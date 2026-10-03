//! 接口集成测试：用真实 Postgres + Redis 驱动完整 HTTP 链路
//!
//! 覆盖单元测试无法观测的边界：数据库迁移、认证会话、权限拦截、事务一致性。
//!
//! 运行方式（无需 Docker）：
//! ```bash
//! scripts/test_env.sh start
//! eval "$(scripts/test_env.sh env)"
//! cargo test --test api_integration -- --ignored --test-threads=1
//! ```
//!
//! 这些用例标记为 `#[ignore]`，因为它们需要真实依赖；
//! CI 通过 `--ignored` 显式运行，`cargo test` 默认只跑单元测试。

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use axum_api::config::{
    AuditLogConfig, Config, DatabaseConfig, MetricsConfig, RateLimitConfig, RedisConfig,
    SecurityConfig,
};
use axum_api::middleware::api_metrics::{EndpointMetric, MetricsCollector};
use axum_api::repository::audit_log::AuditLogRepository;
use axum_api::router::create_router;
use axum_api::utils::redis::RedisClient;

// ──────────────────────────────────────────────
// 环境与脚手架
// ──────────────────────────────────────────────

fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").expect(
        "缺少 TEST_DATABASE_URL，请先执行: scripts/test_env.sh start && eval \"$(scripts/test_env.sh env)\"",
    )
}

fn test_redis_url() -> String {
    std::env::var("TEST_REDIS_URL").expect("缺少 TEST_REDIS_URL")
}

fn test_config(login_max_failures: u64) -> Config {
    Config {
        server_addr: "127.0.0.1:0".parse().unwrap(),
        jwt_secret: std::env::var("TEST_JWT_SECRET")
            .unwrap_or_else(|_| "integration-test-secret-value-0123456789".to_string()),
        jwt_expiration_seconds: 3600,
        cors_allowed_origins: vec!["http://localhost:3000".to_string()],
        redis: RedisConfig {
            url: test_redis_url(),
        },
        // 测试内请求较多，放宽限流阈值避免相互干扰
        rate_limit: RateLimitConfig {
            ip_max_requests: 100_000,
            ip_window_seconds: 60,
            user_max_requests: 100_000,
            user_window_seconds: 60,
        },
        security: SecurityConfig {
            trust_proxy_headers: false,
            login_max_failures,
            login_failure_window_seconds: 300,
        },
        database: DatabaseConfig {
            write_url: test_database_url(),
            max_size: 10,
            connect_timeout_seconds: 10,
        },
        // 清理任务由 main 启动，测试不启动；但字段仍要给值
        audit_log: AuditLogConfig {
            retention_days: 90,
            cleanup_interval_seconds: 3600,
            cleanup_batch_size: 100,
            cleanup_max_batches: 5,
        },
        metrics: MetricsConfig {
            // 1 秒：测试里等一次 flush 很快
            flush_interval_seconds: 1,
            key_ttl_seconds: 600,
            max_buffered_endpoints: 1000,
        },
        migrate_on_startup: true,
    }
}

/// 每个用例构建独立路由
///
/// 连接管理器（Redis/DB）绑定创建它的 Tokio runtime，
/// 而 `#[tokio::test]` 每个用例都会新建 runtime，因此不能跨用例共享路由。
async fn app() -> Router {
    // 后台任务（指标 flush、审计清理）由 main 启动，测试不启动它们，
    // 否则会残留一批任务持续打共享测试库与 Redis
    create_router(test_config(1_000))
        .await
        .map(|(router, _state)| router)
        .expect("构建路由失败")
}

fn request(method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(req).await.expect("请求执行失败");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("读取响应体失败");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn login(app: &Router, username: &str, password: &str) -> (StatusCode, Value) {
    send(
        app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": username, "password": password })),
        ),
    )
    .await
}

async fn login_token(app: &Router, username: &str, password: &str) -> String {
    let (status, body) = login(app, username, password).await;
    assert_eq!(status, StatusCode::OK, "登录失败: {body}");
    body["data"]["token"]
        .as_str()
        .expect("响应缺少 token")
        .to_string()
}

/// 清掉"首次登录强制改密"标记
///
/// 管理员建号时口令由管理员代选，产品要求该用户下次登录后自行改掉（v0.11.0）。
/// 多数权限用例并不关心改密流程，只关心"这个账号能否调某个接口"，
/// 因此这里**显式**清标记，让这些用例继续测它们本来要测的东西。
///
/// 为什么不塞进 `login_token`：那会让"登录"这个动作产生改库的副作用，
/// 用例便不再知道自己依赖了这个前提——正是 v0.9.0 记过的
/// "测试自己也会说谎"。强制改密本身由 v0.11.0 专项用例完整覆盖。
async fn clear_must_change_password(username: &str) {
    sqlx::query("UPDATE users SET must_change_password = FALSE WHERE username = $1")
        .bind(username)
        .execute(&pool().await)
        .await
        .expect("清除强制改密标记失败");
}

/// 建号 → 激活 → 拿到**可正常调用接口**的令牌
///
/// 绝大多数用例要的是这个，而不是"受限令牌"。
async fn activated_token(app: &Router, username: &str, password: &str) -> String {
    clear_must_change_password(username).await;
    login_token(app, username, password).await
}

async fn admin_token(app: &Router) -> String {
    login_token(app, "admin", "admin123").await
}

/// 生成唯一用户名，避免测试之间互相影响（测试库长期存在）
fn unique(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &id[..8])
}

// ──────────────────────────────────────────────
// 启动与迁移
// ──────────────────────────────────────────────

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn empty_database_is_migrated_on_boot() {
    let app = app().await;

    // 路由能构建成功即说明 create_router 内的迁移与种子数据没有失败
    let (status, body) = send(&app, request("GET", "/api/health", None, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["status"], "ok");
    assert_eq!(body["data"]["database"], "up");
    assert_eq!(body["data"]["redis"], "up");

    // 用真实连接确认关键表由迁移创建
    let pool = sqlx::PgPool::connect(&test_database_url()).await.unwrap();
    for table in [
        "users",
        "roles",
        "user_roles",
        "audit_logs",
        "menus",
        "dict_types",
    ] {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists.0, "迁移后缺少表: {table}");
    }
}

// ──────────────────────────────────────────────
// 认证与会话
// ──────────────────────────────────────────────

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn protected_route_requires_token() {
    let app = app().await;
    let (status, _) = send(&app, request("GET", "/api/auth/me", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn login_rejects_wrong_password_with_401() {
    let app = app().await;
    let (status, body) = login(&app, "admin", "definitely-not-the-password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn logout_invalidates_only_the_current_token() {
    let app = app().await;

    let first = login_token(&app, "admin", "admin123").await;
    let second = login_token(&app, "admin", "admin123").await;

    let (status, _) = send(
        &app,
        request("POST", "/api/auth/logout", Some(&first), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (first_status, _) = send(&app, request("GET", "/api/auth/me", Some(&first), None)).await;
    assert_eq!(first_status, StatusCode::UNAUTHORIZED, "已注销令牌必须失效");

    let (second_status, _) = send(&app, request("GET", "/api/auth/me", Some(&second), None)).await;
    assert_eq!(second_status, StatusCode::OK, "其他设备令牌不应受影响");

    // 回归：新登录不得让已注销的旧令牌复活
    let third = login_token(&app, "admin", "admin123").await;
    let (first_again, _) = send(&app, request("GET", "/api/auth/me", Some(&first), None)).await;
    assert_eq!(
        first_again,
        StatusCode::UNAUTHORIZED,
        "重新登录后旧令牌仍必须保持失效"
    );
    let (third_status, _) = send(&app, request("GET", "/api/auth/me", Some(&third), None)).await;
    assert_eq!(third_status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn repeated_login_failures_are_locked_out() {
    // 需要低阈值，单独构建路由，避免影响其他用例
    let (strict, _state) = create_router(test_config(3)).await.unwrap();
    let username = unique("lockout_probe");

    for _ in 0..3 {
        let _ = login(&strict, &username, "wrong-password").await;
    }

    let (status, body) = login(&strict, &username, "wrong-password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

// ──────────────────────────────────────────────
// 权限与数据一致性
// ──────────────────────────────────────────────

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn non_admin_is_blocked_from_admin_api() {
    let app = app().await;
    let username = unique("plain_user");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let token = activated_token(&app, &username, "user1234").await;

    // 普通用户可读取字典（通用展示数据）
    let (dict_status, _) = send(
        &app,
        request("GET", "/api/dict/status/items", Some(&token), None),
    )
    .await;
    assert_ne!(dict_status, StatusCode::FORBIDDEN, "字典读取不应要求 admin");

    // 但不能访问管理接口。
    //
    // v0.5.0 PR-3 之前这里被 `require_role("admin")` 拦住；那道闸门已删除，
    // 现在拦住它的是**权限码**——注册得到的账号没有任何授权，守卫在提取阶段 403。
    // 判定依据从"角色名"换成了"权限码"，对外表现不变。
    let (admin_status, _) =
        send(&app, request("GET", "/api/admin/users", Some(&token), None)).await;
    assert_eq!(admin_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn admin_user_crud_projects_roles_consistently() {
    let app = app().await;
    let token = admin_token(&app).await;
    let username = unique("crud_user");
    let email = format!("{username}@example.com");

    // 创建：响应必须带上真实角色集合（历史上恒为空）
    let (status, created) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": username,
                "email": email,
                "password": "user1234",
                "role": "user"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let user_id = created["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["data"]["role"], "user");
    assert_eq!(created["data"]["roles"], json!(["user"]));

    // 列表：角色同样不能为空
    let (_, list) = send(
        &app,
        request(
            "GET",
            "/api/admin/users?page=1&page_size=200",
            Some(&token),
            None,
        ),
    )
    .await;
    let listed = list["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == username)
        .expect("新建用户应出现在列表中");
    assert_eq!(listed["roles"], json!(["user"]));

    // 改角色：以 user_roles 为准，响应反映新角色
    let (status, updated) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{user_id}"),
            Some(&token),
            Some(json!({
                "username": username,
                "email": email,
                "role": "admin",
                "is_active": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["data"]["role"], "admin");
    assert_eq!(updated["data"]["roles"], json!(["admin"]));

    // 删除
    let (status, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/users/{user_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 删除后 user_roles 由外键级联清理
    let pool = sqlx::PgPool::connect(&test_database_url()).await.unwrap();
    let leftovers: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user_roles WHERE user_id = $1")
        .bind(uuid::Uuid::parse_str(&user_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(leftovers.0, 0, "删除用户后不应残留角色关联");
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn last_admin_cannot_be_demoted_or_deleted() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (_, me) = send(&app, request("GET", "/api/auth/me", Some(&token), None)).await;
    let admin_id = me["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{admin_id}"),
            Some(&token),
            Some(json!({
                "username": "admin",
                "email": "admin@example.com",
                "role": "user",
                "is_active": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/users/{admin_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn admin_requests_are_written_to_audit_log() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 制造一条可识别的请求
    send(&app, request("GET", "/api/admin/roles", Some(&token), None)).await;

    // 审计写入是异步的，允许短暂延迟
    for _ in 0..20 {
        let (_, logs) = send(
            &app,
            request(
                "GET",
                "/api/admin/audit-logs?page=1&page_size=5",
                Some(&token),
                None,
            ),
        )
        .await;
        if logs["data"]["total"].as_i64().unwrap_or(0) > 0 {
            assert!(
                logs["data"]["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|it| it["method"] == "GET"),
                "审计记录应包含方法与路径: {logs}"
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    panic!("操作日志未被写入（audit_logs 表为空）");
}

// ──────────────────────────────────────────────
// 菜单驱动的导航
// ──────────────────────────────────────────────

/// 递归收集菜单树里的所有名称
fn collect_menu_names(value: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(nodes) = value.as_array() {
        for node in nodes {
            if let Some(name) = node["name"].as_str() {
                names.push(name.to_string());
            }
            names.extend(collect_menu_names(&node["children"]));
        }
    }
    names
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn current_user_menu_tree_follows_role_assignment() {
    let app = app().await;

    // 未登录不可读取
    let (status, _) = send(&app, request("GET", "/api/auth/menus", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // admin：包含系统管理及其子页面
    let admin = admin_token(&app).await;
    let (status, body) = send(&app, request("GET", "/api/auth/menus", Some(&admin), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let admin_menus = collect_menu_names(&body["data"]);
    assert!(!admin_menus.is_empty(), "菜单种子未生效: {body}");
    for expected in ["首页", "组件示例", "系统管理", "用户管理", "菜单管理"] {
        assert!(
            admin_menus.iter().any(|n| n == expected),
            "admin 应看到「{expected}」: {admin_menus:?}"
        );
    }

    // 普通用户：只有通用页面，不含系统管理
    let username = unique("menu_user");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let user = activated_token(&app, &username, "user1234").await;
    let (status, body) = send(&app, request("GET", "/api/auth/menus", Some(&user), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let user_menus = collect_menu_names(&body["data"]);

    assert!(
        user_menus.iter().any(|n| n == "首页"),
        "普通用户应看到首页: {user_menus:?}"
    );
    assert!(
        !user_menus.iter().any(|n| n == "系统管理"),
        "普通用户不应看到系统管理: {user_menus:?}"
    );
    // 不同角色拿到的菜单必须真的有差异，而不是都返回全量
    assert!(user_menus.len() < admin_menus.len());
}

// ──────────────────────────────────────────────
// 接口文档契约
// ──────────────────────────────────────────────

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn every_documented_route_is_implemented() {
    let app = app().await;
    let routes = axum_api::docs::documented_paths();
    assert!(!routes.is_empty(), "OpenAPI 文档不应为空");

    let mut missing = Vec::new();

    for (path, methods) in routes {
        // 路径参数替换为合法值，避免因参数解析失败而误判
        let concrete = path
            .replace("{id}", "00000000-0000-0000-0000-000000000000")
            .replace("{user_id}", "00000000-0000-0000-0000-000000000000")
            .replace("{role_id}", "00000000-0000-0000-0000-000000000000")
            .replace("{code}", "probe")
            .replace("{*path}", "index.html");

        for method in methods {
            let (status, _) = send(&app, request(&method, &concrete, None, None)).await;
            if status == StatusCode::NOT_FOUND {
                missing.push(format!("{method} {path}"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "以下接口已写入 OpenAPI 文档但实际不存在: {missing:?}"
    );
}

// ──────────────────────────────────────────────
// 权限码（menus.permission 从元数据变为强制拦截）
// ──────────────────────────────────────────────

/// 测试库连接（直接操作数据，验证授权变更的效果）
async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&test_database_url())
        .await
        .expect("连接测试库失败")
}

/// 撤销角色对某个权限码的授权，返回是否确实删除了授权行
async fn revoke_permission_code(role_name: &str, code: &str) -> bool {
    let result = sqlx::query(
        "DELETE FROM role_menus rm \
         USING menus m, roles r \
         WHERE rm.menu_id = m.id AND rm.role_id = r.id \
           AND r.name = $1 AND m.permission = $2",
    )
    .bind(role_name)
    .bind(code)
    .execute(&pool().await)
    .await
    .expect("撤销权限码失败");
    result.rows_affected() > 0
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn permission_codes_are_seeded_and_returned_to_admin() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let codes: Vec<String> = serde_json::from_value(body["data"].clone()).expect("权限码格式错误");

    // 种子必须覆盖权限码定义表里的每一条，且权限码唯一
    let declared: Vec<&str> = axum_api::model::permission::PERMISSION_DEFS
        .iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(
        codes.len(),
        declared.len(),
        "admin 拿到的权限码数量应与定义表一致"
    );
    for code in &declared {
        assert!(
            codes.iter().any(|c| c == code),
            "admin 缺少权限码 {code}（实际: {codes:?}）"
        );
    }
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn ordinary_user_has_no_permission_codes() {
    let app = app().await;
    let username = unique("perm_user");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let token = activated_token(&app, &username, "user1234").await;
    let (status, body) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"].as_array().map(|a| a.len()),
        Some(0),
        "普通用户不应持有任何权限码"
    );
}

/// **核心用例**：权限码不只是元数据，撤销后接口立即 403。
///
/// 这是 v0.3.0「已知限制」里点名的那条差距，本用例是它的回归防线。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn revoked_permission_code_blocks_the_interface() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 先确认撤销前可用
    let (before, body) = send(&app, request("GET", "/api/admin/users", Some(&token), None)).await;
    assert_eq!(before, StatusCode::OK, "撤销前应可访问: {body}");

    assert!(
        revoke_permission_code("admin", axum_api::model::permission::USER_LIST).await,
        "撤销前应存在 admin 的 system:user:list 授权"
    );

    // 撤销后同一接口立即 403（无缓存窗口）
    let (after, body) = send(&app, request("GET", "/api/admin/users", Some(&token), None)).await;
    assert_eq!(after, StatusCode::FORBIDDEN, "撤销权限码后应被拦截: {body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("system:user:list"),
        "403 应指明缺失的权限码，便于排查: {body}"
    );

    // 未被撤销的接口不受影响，证明拦截是按权限码精确到接口的
    let (roles_status, roles_body) =
        send(&app, request("GET", "/api/admin/roles", Some(&token), None)).await;
    assert_eq!(
        roles_status,
        StatusCode::OK,
        "撤销 system:user:list 不应影响 system:role:list: {roles_body}"
    );

    // 恢复授权，避免影响其他用例
    let pool = pool().await;
    sqlx::query(
        "INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, m.id FROM roles r, menus m \
         WHERE r.name = 'admin' AND m.permission = $1 \
         ON CONFLICT (role_id, menu_id) DO NOTHING",
    )
    .bind(axum_api::model::permission::USER_LIST)
    .execute(&pool)
    .await
    .expect("恢复权限码授权失败");
}

/// **鉴权必须早于入参校验**：无权限 + 畸形请求体也必须返回 403，而不是 422。
///
/// 曾经的缺陷：把校验写在 handler 函数体里，而 axum 先解析 `Json`，
/// 于是缺字段的请求拿到 422，等于把接口的参数结构反馈给了无权限调用者。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn permission_is_checked_before_body_validation() {
    let app = app().await;
    let token = admin_token(&app).await;

    assert!(revoke_permission_code("admin", axum_api::model::permission::USER_DELETE).await);

    // 请求体字段名写错（真实字段是 ids），若先解析请求体会得到 422
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users/batch-delete",
            Some(&token),
            Some(json!({ "wrong_field": [] })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "应先鉴权后校验入参，实际响应: {body}"
    );

    let pool = pool().await;
    sqlx::query(
        "INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, m.id FROM roles r, menus m \
         WHERE r.name = 'admin' AND m.permission = $1 \
         ON CONFLICT (role_id, menu_id) DO NOTHING",
    )
    .bind(axum_api::model::permission::USER_DELETE)
    .execute(&pool)
    .await
    .expect("恢复权限码授权失败");
}

/// 权限码授权的增删改接口本身也受权限码保护（防止越权授予自己权限）
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn permission_grant_endpoint_requires_grant_code() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (before, _) = send(
        &app,
        request(
            "PUT",
            "/api/admin/roles/00000000-0000-0000-0000-000000000000/menus",
            Some(&token),
            Some(json!({ "menu_ids": [] })),
        ),
    )
    .await;
    // 角色不存在会返回 404/404 类错误，但绝不能是 200，也不能是权限码 403
    assert_ne!(before, StatusCode::FORBIDDEN, "撤销前不应被权限码拦截");

    assert!(revoke_permission_code("admin", axum_api::model::permission::MENU_GRANT).await);

    let (after, body) = send(
        &app,
        request(
            "PUT",
            "/api/admin/roles/00000000-0000-0000-0000-000000000000/menus",
            Some(&token),
            Some(json!({ "menu_ids": [] })),
        ),
    )
    .await;
    assert_eq!(
        after,
        StatusCode::FORBIDDEN,
        "撤销 system:menu:grant 后授权接口应 403: {body}"
    );

    let pool = pool().await;
    sqlx::query(
        "INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, m.id FROM roles r, menus m \
         WHERE r.name = 'admin' AND m.permission = $1 \
         ON CONFLICT (role_id, menu_id) DO NOTHING",
    )
    .bind(axum_api::model::permission::MENU_GRANT)
    .execute(&pool)
    .await
    .expect("恢复权限码授权失败");
}

// ──────────────────────────────────────────────
// 角色与授权写入：写路径必须"要么完整成功、要么整体失败"
// ──────────────────────────────────────────────

async fn role_id_by_name(name: &str) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM roles WHERE name = $1")
        .bind(name)
        .fetch_one(&pool().await)
        .await
        .unwrap_or_else(|e| panic!("查询角色 {name} 失败: {e}"))
}

/// 某权限码对应的菜单 ID（权限码即按钮型菜单行的 `menus.permission`）
async fn menu_id_of(code: &str) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM menus WHERE permission = $1")
        .bind(code)
        .fetch_one(&pool().await)
        .await
        .unwrap_or_else(|e| panic!("查询权限码 {code} 对应菜单失败: {e}"))
}

async fn granted_menu_ids(role_id: uuid::Uuid) -> Vec<uuid::Uuid> {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT menu_id FROM role_menus WHERE role_id = $1")
        .bind(role_id)
        .fetch_all(&pool().await)
        .await
        .expect("查询角色授权失败")
}

async fn role_still_exists(id: uuid::Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM roles WHERE id = $1)")
        .bind(id)
        .fetch_one(&pool().await)
        .await
        .expect("查询角色失败")
}

async fn menu_still_exists(id: uuid::Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM menus WHERE id = $1)")
        .bind(id)
        .fetch_one(&pool().await)
        .await
        .expect("查询菜单失败")
}

async fn grant_count_for_menu(menu_id: uuid::Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM role_menus WHERE menu_id = $1")
        .bind(menu_id)
        .fetch_one(&pool().await)
        .await
        .expect("查询授权失败")
}

/// 通过接口新建角色，返回其 ID
async fn create_role_via_api(app: &Router, token: &str, name: &str) -> uuid::Uuid {
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/roles",
            Some(token),
            Some(json!({ "name": name, "description": "v0.5.0 测试角色" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新建角色失败: {body}");
    body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("新建角色响应里没有可解析的 id: {body}"))
}

async fn assign_menus(
    app: &Router,
    token: &str,
    role_id: uuid::Uuid,
    menu_ids: &[uuid::Uuid],
) -> (StatusCode, Value) {
    send(
        app,
        request(
            "PUT",
            &format!("/api/admin/roles/{role_id}/menus"),
            Some(token),
            Some(json!({ "menu_ids": menu_ids })),
        ),
    )
    .await
}

async fn delete_role(app: &Router, token: &str, role_id: uuid::Uuid) -> (StatusCode, Value) {
    send(
        app,
        request(
            "DELETE",
            &format!("/api/admin/roles/{role_id}"),
            Some(token),
            None,
        ),
    )
    .await
}

/// 通过接口撤销权限，**必须真的生效**。
///
/// 曾经的缺陷：`assign_role_menus` 对 DELETE 与 INSERT 都用 `.ok()` 吞掉错误却仍提交，
/// 于是"取消勾选 → 保存"可能静默失效——UI 上看着撤销成功，权限其实还在。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn revoking_permission_through_the_api_actually_takes_effect() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_id = create_role_via_api(&app, &token, &unique("role_revoke")).await;

    let user_list = menu_id_of(axum_api::model::permission::USER_LIST).await;
    let role_list = menu_id_of(axum_api::model::permission::ROLE_LIST).await;

    let (status, body) = assign_menus(&app, &token, role_id, &[user_list, role_list]).await;
    assert_eq!(status, StatusCode::OK, "首次授权应成功: {body}");
    let granted = granted_menu_ids(role_id).await;
    assert!(granted.contains(&user_list) && granted.contains(&role_list));

    // 全量替换语义：只提交 user:list，role:list 必须被撤销
    let (status, body) = assign_menus(&app, &token, role_id, &[user_list]).await;
    assert_eq!(status, StatusCode::OK, "重新授权应成功: {body}");

    let granted = granted_menu_ids(role_id).await;
    assert!(
        granted.contains(&user_list),
        "仍在提交集合中的权限码应保留: {granted:?}"
    );
    assert!(
        !granted.contains(&role_list),
        "未提交的权限码必须被撤销（曾因吞错而静默失效）: {granted:?}"
    );

    let _ = delete_role(&app, &token, role_id).await;
}

/// 传入不存在的菜单 ID 必须**整体失败**，不能静默部分授权后还返回成功。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn assigning_an_unknown_menu_id_is_rejected_atomically() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_id = create_role_via_api(&app, &token, &unique("role_badid")).await;

    let good = menu_id_of(axum_api::model::permission::USER_LIST).await;
    let bogus = uuid::Uuid::new_v4();

    let (status, body) = assign_menus(&app, &token, role_id, &[good, bogus]).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "含非法菜单 ID 时应明确报错，而不是返回成功: {body}"
    );
    assert!(
        granted_menu_ids(role_id).await.is_empty(),
        "事务必须整体回滚：非法 ID 不应导致合法的那个被写入"
    );

    let _ = delete_role(&app, &token, role_id).await;
}

/// 授权给不存在的角色应是 404，而不是"成功"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn assigning_menus_to_an_unknown_role_is_not_found() {
    let app = app().await;
    let token = admin_token(&app).await;
    let good = menu_id_of(axum_api::model::permission::USER_LIST).await;

    let (status, body) = assign_menus(&app, &token, uuid::Uuid::new_v4(), &[good]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "应返回 404: {body}");
}

/// 内置角色不可删除：角色种子只在 `roles` 表为空时写入，删掉不会被重建。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn builtin_roles_cannot_be_deleted() {
    let app = app().await;
    let token = admin_token(&app).await;

    for name in ["admin", "user"] {
        let id = role_id_by_name(name).await;
        let (status, body) = delete_role(&app, &token, id).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "内置角色 {name} 不应可删除: {body}"
        );
        assert!(role_still_exists(id).await, "内置角色 {name} 必须仍然存在");
    }
}

/// 仍有用户持有该角色时必须拒绝删除。
///
/// `user_roles.role_id` 是 `ON DELETE CASCADE`：直接删角色会**静默**剥掉这些用户的角色，
/// 用户变成"没有任何角色"却毫不知情。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_role_still_held_by_users_is_refused() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_name = unique("role_inuse");
    let role_id = create_role_via_api(&app, &token, &role_name).await;

    // 经用户表单分配该自定义角色（v0.5.0 PR-2 起自定义角色可分配）
    let username = unique("user_inuse");
    let (status, created) = create_user_via_api(&app, &token, &username, &role_name).await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {created}");
    let user_id = uuid::Uuid::parse_str(created["data"]["id"].as_str().unwrap()).unwrap();

    // 该用户确实持有这个自定义角色（否则"被占用"的前提不成立）
    let held: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM user_roles WHERE user_id = $1 AND role_id = $2")
            .bind(user_id)
            .bind(role_id)
            .fetch_one(&pool().await)
            .await
            .expect("查询角色占用失败");
    assert_eq!(held.0, 1, "测试前提：用户应持有该自定义角色");

    let (status, body) = delete_role(&app, &token, role_id).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "仍被用户占用的角色不应可删除: {body}"
    );
    assert!(
        body["message"].as_str().unwrap_or("").contains('1'),
        "错误应说明还有多少用户在用: {body}"
    );
    assert!(
        role_still_exists(role_id).await,
        "角色必须还在，且用户的角色关系未被级联剥掉"
    );

    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(role_id)
        .execute(&pool().await)
        .await
        .expect("清理测试角色失败");
}

/// 无人使用的自定义角色可以正常删除
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_an_unused_custom_role_succeeds() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_id = create_role_via_api(&app, &token, &unique("role_free")).await;

    let (status, body) = delete_role(&app, &token, role_id).await;
    assert_eq!(status, StatusCode::OK, "删除空闲角色应成功: {body}");
    assert!(!role_still_exists(role_id).await, "角色应已从表中删除");
}

/// 删除不存在的角色返回 404，而不是"删除成功"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_an_unknown_role_is_not_found() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (status, body) = delete_role(&app, &token, uuid::Uuid::new_v4()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "应返回 404: {body}");
}

/// 删除菜单会连同其下嵌套的权限码按钮一起消失（依赖外键级联）
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_menu_removes_its_nested_permission_buttons() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(json!({
                "name": unique("tmp_dir"),
                "type": "directory",
                "path": format!("/{}", unique("tmp")),
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建测试菜单失败: {body}");
    let parent_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(json!({
                "parent_id": parent_id,
                "name": "临时按钮",
                "type": "button",
                "permission": format!("tmp:test:{}", &unique("p")[5..]),
                "sort_order": 1
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建测试按钮失败: {body}");
    let child_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    // 授权给 admin，确保 role_menus 也需要一并清理
    sqlx::query(
        "INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, $1 FROM roles r WHERE r.name = 'admin' ON CONFLICT DO NOTHING",
    )
    .bind(child_id)
    .execute(&pool().await)
    .await
    .expect("构造授权失败");

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{parent_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "删除菜单失败: {body}");

    assert!(!menu_still_exists(parent_id).await, "父菜单应被删除");
    assert!(
        !menu_still_exists(child_id).await,
        "嵌套的权限码按钮应随级联一并消失"
    );
    assert_eq!(
        grant_count_for_menu(child_id).await,
        0,
        "角色授权应随级联一并清理"
    );
}

/// 删除不存在的菜单返回 404，而不是"删除成功"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_an_unknown_menu_is_not_found() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{}", uuid::Uuid::new_v4()),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "应返回 404: {body}");
}

/// 回归：`GET /api/admin/menus?role_id=` 曾对「只勾了子菜单、没勾上级目录」的角色
/// 返回**空树**。
///
/// 早先的 `build_tree` 只把 `parent_id IS NULL` 当根，父节点不在授权集合里的节点
/// 被悄悄丢弃。于是授权弹窗显示「该角色没有任何权限」，管理员一保存就把授权全清空
/// ——静默的数据丢失。admin 因被种子授满全部菜单（含所有祖先）而恰好看不出问题。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn role_menu_query_keeps_grants_whose_ancestors_are_not_granted() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 造三层临时菜单：目录 → 页面 → 按钮
    let make_menu = |parent: Option<uuid::Uuid>, name: String, kind: &str| {
        let mut body = json!({
            "name": name,
            "type": kind,
            "sort_order": 97
        });
        if let Some(p) = parent {
            body["parent_id"] = json!(p);
        }
        if kind == "button" {
            body["permission"] = json!(format!("tmp:test:{}", &unique("p")[5..]));
        } else {
            body["path"] = json!(format!("/{}", unique("t")));
        }
        body
    };

    let dir_name = unique("tmp_dir");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(make_menu(None, dir_name.clone(), "directory")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时目录失败: {body}");
    let dir_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    let page_name = unique("tmp_page");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(make_menu(Some(dir_id), page_name.clone(), "menu")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时页面失败: {body}");
    let page_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    let btn_name = unique("tmp_btn");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(make_menu(Some(page_id), btn_name.clone(), "button")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时按钮失败: {body}");
    let btn_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    let role_id = create_role_via_api(&app, &token, &unique("tmp_role")).await;

    // 场景 A：只授权按钮，页面与目录都没授权
    let (status, body) = assign_menus(&app, &token, role_id, &[btn_id]).await;
    assert_eq!(status, StatusCode::OK, "授权按钮失败: {body}");

    let (status, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/admin/menus?role_id={role_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names = collect_menu_names(&body["data"]);
    assert_eq!(
        names.len(),
        1,
        "只授权一个菜单时应只返回它，实际: {names:?}"
    );
    assert_eq!(
        names.first().map(String::as_str),
        Some(btn_name.as_str()),
        "只授权按钮时 ?role_id= 必须返回该按钮（早先返回空树）: {names:?}"
    );

    // 场景 B：授权页面 + 按钮，页面应作为根返回、按钮挂在它下面
    let (status, body) = assign_menus(&app, &token, role_id, &[page_id, btn_id]).await;
    assert_eq!(status, StatusCode::OK, "授权页面+按钮失败: {body}");

    let (status, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/admin/menus?role_id={role_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let roots = body["data"].as_array().expect("data 应为数组");
    assert_eq!(
        roots.len(),
        1,
        "页面未授权其父目录，应作为根返回: {roots:?}"
    );
    assert_eq!(roots[0]["name"].as_str(), Some(page_name.as_str()));
    let children = roots[0]["children"].as_array().expect("children 应为数组");
    assert_eq!(children.len(), 1, "按钮应挂在页面下: {children:?}");
    assert_eq!(children[0]["name"].as_str(), Some(btn_name.as_str()));

    // 清理：**先撤授权，再删角色**。顺序不是随意的。
    //
    // v0.9.0 给 `delete_role` 补了授权天花板后，删除一个承载"调用方未持有的码"
    // 的角色会被拒（与 v0.8.0 的 `delete_menu` 同一态度）。本测试的角色带着
    // 自造的 `tmp:test:*` 码，admin 并不持有它，所以必须先把码撤掉，
    // 角色退化成"无码角色"才可删。
    //
    // 这条断言顺带把该约束钉在了集成测试里：将来若有人放宽 delete_role 的
    // 天花板，这条清理会先变红，提醒他确认是不是有意的。
    let (status, body) = assign_menus(&app, &token, role_id, &[]).await;
    assert_eq!(status, StatusCode::OK, "撤销临时角色授权失败: {body}");

    // 角色无用户占用，可直接删；删目录会级联清掉页面与按钮
    let (status, body) = delete_role(&app, &token, role_id).await;
    assert_eq!(status, StatusCode::OK, "清理临时角色失败: {body}");
    let (status, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理临时目录失败");
}

// ──────────────────────────────────────────────
// 角色即数据：自定义角色可分配（v0.5.0 PR-2 拆掉 ASSIGNABLE_ROLES）
// ──────────────────────────────────────────────

/// 建用户请求（角色可任意指定，由被测用例决定）
fn user_payload(username: &str, role: &str) -> Value {
    json!({
        "username": username,
        "email": format!("{username}@example.com"),
        "password": "user1234",
        "role": role,
    })
}

async fn create_user_via_api(
    app: &Router,
    token: &str,
    username: &str,
    role: &str,
) -> (StatusCode, Value) {
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/users",
            Some(token),
            Some(user_payload(username, role)),
        ),
    )
    .await;
    // 管理员建号会置"强制改密"，这里清掉以便后续用例正常调用接口。
    // 见 clear_must_change_password 的说明
    if status == StatusCode::OK {
        clear_must_change_password(username).await;
    }
    (status, body)
}

async fn user_exists(username: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE username = $1)")
        .bind(username)
        .fetch_one(&pool().await)
        .await
        .expect("查询用户失败")
}

async fn email_in_db(username: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&pool().await)
        .await
        .expect("查询用户失败")
}

async fn update_role_via_api(
    app: &Router,
    token: &str,
    role_id: uuid::Uuid,
    name: &str,
) -> (StatusCode, Value) {
    send(
        app,
        request(
            "PUT",
            &format!("/api/admin/roles/{role_id}"),
            Some(token),
            Some(json!({ "name": name, "description": "PR-2 测试" })),
        ),
    )
    .await
}

async fn role_name_in_db(role_id: uuid::Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT name FROM roles WHERE id = $1")
        .bind(role_id)
        .fetch_one(&pool().await)
        .await
        .expect("查询角色名失败")
}

/// PR-2 的核心：角色是数据，新建一个角色立刻就能通过用户表单分配。
/// 拆白名单之前这里是 400，自定义角色永远建不出可用的人。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn custom_role_can_be_assigned_to_a_new_user() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_name = unique("auditor");
    let role_id = create_role_via_api(&app, &token, &role_name).await;

    let username = unique("user_auditor");
    let (status, body) = create_user_via_api(&app, &token, &username, &role_name).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "自定义角色应可分配给新建用户: {body}"
    );
    // 注意：`UserInfo.role` 是只有 admin/user 两值的展示用枚举，
    // 自定义角色在这里一律塌缩成 "user"。真正的角色集合在 `roles`。
    assert_eq!(
        body["data"]["roles"][0].as_str(),
        Some(role_name.as_str()),
        "响应里的角色集合应是自定义角色: {body}"
    );

    // 角色关系确实落库（不只是响应里好看）
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();
    let (status, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/admin/users/{user_id}/roles"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let roles: Vec<&str> = body["data"]
        .as_array()
        .expect("data 应为数组")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(roles, vec![role_name.as_str()], "{body}");

    let _ = delete_role(&app, &token, role_id).await;
}

/// 改一个已有用户的角色同样要接受自定义角色（用户表单的 update 路径）。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn custom_role_can_be_assigned_when_updating_a_user() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_name = unique("operator");
    let role_id = create_role_via_api(&app, &token, &role_name).await;

    let username = unique("user_promote");
    let (status, body) = create_user_via_api(&app, &token, &username, "user").await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {body}");
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{user_id}"),
            Some(&token),
            Some(user_payload(&username, &role_name)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "更新用户时应接受自定义角色: {body}");

    let (status, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/admin/users/{user_id}/roles"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let roles: Vec<&str> = body["data"]
        .as_array()
        .expect("data 应为数组")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(roles, vec![role_name.as_str()], "{body}");

    let _ = delete_role(&app, &token, role_id).await;
}

/// 不存在的角色必须被拒绝，**且不能留下半成品用户**。
///
/// `create_user` 先建用户行、再调 `replace_user_roles` 校验角色，两者不在同一事务。
/// 拆掉白名单后这条路径才真正可达：校验若放在写入之后，就会返回一个 400
/// 同时在库里留下一个"没有任何角色"的用户。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn unknown_role_is_rejected_without_leaving_a_half_created_user() {
    let app = app().await;
    let token = admin_token(&app).await;
    let username = unique("user_ghost");

    let (status, body) = create_user_via_api(&app, &token, &username, "ghost_role").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "不存在的角色应返回 400: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("ghost_role"),
        "错误信息应带上实际角色名: {body}"
    );
    assert!(
        !user_exists(&username).await,
        "请求已失败，用户行不得落库（否则库里多出一个没有任何角色的用户）"
    );
}

/// 更新路径同理：角色非法时基础字段也不能被改掉。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn updating_with_unknown_role_changes_nothing() {
    let app = app().await;
    let token = admin_token(&app).await;
    let username = unique("user_ghost2");
    let (status, body) = create_user_via_api(&app, &token, &username, "user").await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {body}");
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();
    let original_email = email_in_db(&username).await;

    // 故意把邮箱换掉：若角色校验发生在写入之后，这个邮箱就会真的被改掉
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{user_id}"),
            Some(&token),
            Some(json!({
                "username": username,
                "email": format!("changed-{username}@example.com"),
                "password": "user1234",
                "role": "ghost_role",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    assert_eq!(
        email_in_db(&username).await,
        original_email,
        "角色校验失败时不得留下已改过邮箱的半成品: {body}"
    );
}

/// 角色名在写入时就归一化，撞名返回 409 而不是 500。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn role_names_are_normalized_on_write_and_conflicts_return_409() {
    let app = app().await;
    let token = admin_token(&app).await;
    let suffix = unique("n").to_uppercase();
    let noisy = format!("  MiXeD_{suffix}  ");
    let canonical = format!("mixed_{}", suffix.to_lowercase());

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/roles",
            Some(&token),
            Some(json!({ "name": noisy, "description": "PR-2" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建角色失败: {body}");
    assert_eq!(
        body["data"]["name"].as_str(),
        Some(canonical.as_str()),
        "角色名应在写入时归一化（trim + 小写）: {body}"
    );
    let role_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    // 归一化后同名 → 409（不是 500）。这里必须真的撞上同一个名字：
    // 建的是 "mixed_<suffix>"，所以第二个名字也得带 MIXED_ 前缀。
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/roles",
            Some(&token),
            Some(json!({ "name": format!("MIXED_{}", suffix.to_lowercase()), "description": "PR-2" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "归一化后同名应返回 409: {body}"
    );

    // 非法名字是 400，不是 500
    for bad in ["   ", "tab\there", &"a".repeat(51)] {
        let (status, body) = send(
            &app,
            request(
                "POST",
                "/api/admin/roles",
                Some(&token),
                Some(json!({ "name": bad, "description": "PR-2" })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "非法角色名 {bad:?} 应返回 400: {body}"
        );
    }

    // 中间有空格的角色名是合法的（存量库里有这种角色），不能误伤
    let spaced = unique("senior auditor");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/roles",
            Some(&token),
            Some(json!({ "name": spaced, "description": "PR-2" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "中间有空格的角色名应被接受: {body}");
    assert_eq!(body["data"]["name"].as_str(), Some(spaced.as_str()));
    let spaced_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();
    let _ = delete_role(&app, &token, spaced_id).await;

    // 归一化后的名字可直接用于分配用户
    let username = unique("user_mixed");
    let (status, body) = create_user_via_api(&app, &token, &username, &noisy).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "表单传带空格大写的角色名也应命中归一化后的角色: {body}"
    );

    let _ = delete_role(&app, &token, role_id).await;
}

/// 内置角色不可改名，也不能把自定义角色改名成内置角色名。
///
/// 与「内置角色不可删除」同源：`ADMIN_ROLE = "admin"` 是最后一名管理员保护
/// （`count_users_with_role`）和权限码种子（`WHERE r.name='admin'`）的查找依据，
/// 一旦改名这些依据全部落空，系统会变成"没人是管理员"。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn builtin_roles_cannot_be_renamed_or_impersonated() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 内置角色改名（含改成自身的小写形式）
    for name in ["admin", "user"] {
        let id = role_id_by_name(name).await;
        for new_name in ["superadmin", &name.to_uppercase()] {
            let (status, body) = update_role_via_api(&app, &token, id, new_name).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "内置角色 {name} 不应可改名为 {new_name}: {body}"
            );
        }
        assert_eq!(
            role_name_in_db(id).await,
            name,
            "内置角色 {name} 的名字不得被改动"
        );
    }

    // 自定义角色改名成内置角色名也要拒绝
    let role_id = create_role_via_api(&app, &token, &unique("impostor")).await;
    for builtin in ["admin", "user"] {
        let (status, body) = update_role_via_api(&app, &token, role_id, builtin).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "不应能改名成内置角色名 {builtin}: {body}"
        );
    }

    // 不存在的角色 → 404
    let (status, body) =
        update_role_via_api(&app, &token, uuid::Uuid::new_v4(), &unique("ghost_role")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let _ = delete_role(&app, &token, role_id).await;
}

/// 更新角色返回**真实行**，不是编造的 created_at / user_count。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn updating_a_role_returns_the_real_row() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_name = unique("counted");
    let role_id = create_role_via_api(&app, &token, &role_name).await;

    let username = unique("user_counted");
    let (status, body) = create_user_via_api(&app, &token, &username, &role_name).await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {body}");

    let (status, updated) = update_role_via_api(&app, &token, role_id, &role_name).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        updated["data"]["user_count"].as_i64(),
        Some(1),
        "该角色有 1 个用户，回读不该编造 user_count=0: {updated}"
    );

    // created_at 必须是数据库里的原值，而不是"刚刚"。
    // 必须显式要 page_size=200：列表默认每页 10 条，而排序是 created_at ASC，
    // 刚建的角色排在末尾，只取首页会找不到它——那是分页语义，不是 bug。
    let (status, list) = send(
        &app,
        request("GET", "/api/admin/roles?page_size=200", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let from_list = list["data"]["items"]
        .as_array()
        .expect("data.items 应为数组（v0.10.0 起角色列表是分页对象）")
        .iter()
        .find(|r| r["id"].as_str() == Some(&role_id.to_string()))
        .expect("角色列表里应找得到刚更新的角色");
    assert_eq!(
        updated["data"]["created_at"].as_str(),
        from_list["created_at"].as_str(),
        "更新响应的 created_at 应等于库里的值，不能是 now(): {updated}"
    );

    let _ = delete_role(&app, &token, role_id).await;
}

/// `POST /users/:id/roles` 与用户表单走同一套归一化。
///
/// 否则同一个角色经表单提交 "admin" 成功、经本接口提交 "Admin" 却 404。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn append_role_endpoint_normalizes_the_role_name() {
    let app = app().await;
    let token = admin_token(&app).await;
    let role_name = unique("appendable");
    let _role_id = create_role_via_api(&app, &token, &role_name).await;

    let username = unique("user_append");
    let (status, body) = create_user_via_api(&app, &token, &username, "user").await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {body}");
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{user_id}/roles"),
            Some(&token),
            Some(json!({ "user_id": user_id, "role_name": format!("  {}  ", role_name.to_uppercase()) })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "追加角色应先归一化再查库: {body}");

    let (status, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/admin/users/{user_id}/roles"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut roles: Vec<String> = body["data"]
        .as_array()
        .expect("data 应为数组")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    roles.sort();
    let mut expected = vec!["user".to_string(), role_name.clone()];
    expected.sort();
    assert_eq!(roles, expected, "{body}");
}

// ──────────────────────────────────────────────
// v0.6.0 PR-1：多角色用户
//
// 数据模型一直是多角色的（`user_roles` 表 + `UserInfo.roles`），
// 但 `UserManageRequest` 只有单数 `role`，而它是**整体替换**语义：
// 前端只回填 `roles[0]`，一保存就把用户其余角色静默删掉。
// 这一组测试钉住"多角色必须能整体提交、整体回显、整体校验"。
// ──────────────────────────────────────────────

/// 直接查库读某用户持有的角色名（按名排序，顺序无关的比较需要它）
async fn role_names_in_db(user_id: uuid::Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
         WHERE ur.user_id = $1 ORDER BY r.name ASC",
    )
    .bind(user_id)
    .fetch_all(&pool().await)
    .await
    .expect("查询用户角色失败")
}

/// 删除测试用户（避免在共享库里留下带自定义角色的账号）
async fn delete_user_via_api(app: &Router, token: &str, user_id: uuid::Uuid) {
    let (status, body) = send(
        app,
        request(
            "DELETE",
            &format!("/api/admin/users/{user_id}"),
            Some(token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "删除测试用户失败: {body}");
}

/// 建号并断言落库的是**全部**角色，随后清理（用户 + 角色）
async fn create_user_with_roles(
    app: &Router,
    token: &str,
    roles: &[String],
) -> (uuid::Uuid, String) {
    let username = unique("multi_role_user");
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/users",
            Some(token),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "roles": roles,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "多角色建号失败: {body}");
    let id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("响应里没有可解析的用户 id: {body}"));
    // 管理员建号会置"强制改密"，这里清掉以便后续用例正常调用接口
    clear_must_change_password(&username).await;
    (id, username)
}

/// 多角色建号：`roles` 里的每个角色都必须真的落库
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn creating_a_user_with_several_roles_assigns_all_of_them() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("role_a");
    let b = unique("role_b");
    let ida = create_role_via_api(&app, &token, &a).await;
    let idb = create_role_via_api(&app, &token, &b).await;

    let (uid, _) = create_user_with_roles(&app, &token, &[a.clone(), b.clone()]).await;

    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(role_names_in_db(uid).await, expected, "两个角色都应落库");

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
    let _ = delete_role(&app, &token, idb).await;
}

/// **核心回归**：整体提交后角色集合不变，且响应把**全部**角色回显出来。
///
/// v0.6.0 之前响应里虽有 `roles`，前端却只取 `roles[0]` 填表单、
/// 再以单数 `role` 整体覆盖提交——用户其余角色被无声删除。
/// 这条断言正是前端多选框"能正确回填"的前提。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn updating_a_user_round_trips_every_role() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("rt_a");
    let b = unique("rt_b");
    let c = unique("rt_c");
    let ida = create_role_via_api(&app, &token, &a).await;
    let idb = create_role_via_api(&app, &token, &b).await;
    let idc = create_role_via_api(&app, &token, &c).await;

    let (uid, username) =
        create_user_with_roles(&app, &token, &[a.clone(), b.clone(), c.clone()]).await;

    // 模拟"管理员打开编辑框、只改了邮箱就保存"：角色原样回传
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{uid}"),
            Some(&token),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "roles": [a, b, c],
                "is_active": true,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "多角色整体更新失败: {body}");

    let echoed: Vec<String> = body["data"]["roles"]
        .as_array()
        .expect("roles 应为数组")
        .iter()
        .map(|v| v.as_str().expect("角色名应为字符串").to_string())
        .collect();
    let mut expected = echoed.clone();
    expected.sort();
    assert_eq!(
        role_names_in_db(uid).await,
        expected,
        "只改邮箱不应丢掉任何角色"
    );
    assert_eq!(
        echoed.len(),
        3,
        "响应必须回显全部 3 个角色，前端多选框要靠它回填: {body}"
    );

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
    let _ = delete_role(&app, &token, idb).await;
    let _ = delete_role(&app, &token, idc).await;
}

/// 空角色集合必须被拒，且**不能**留下"角色被清空"的半成品
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_empty_role_list_is_rejected_and_changes_nothing() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("empty_a");
    let ida = create_role_via_api(&app, &token, &a).await;
    let (uid, _) = create_user_with_roles(&app, &token, std::slice::from_ref(&a)).await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{uid}"),
            Some(&token),
            Some(json!({
                "username": unique("empty_user"),
                "email": format!("{}@example.com", unique("empty_email")),
                "roles": [],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "空角色集合应被拒: {body}");

    assert_eq!(
        role_names_in_db(uid).await,
        vec![a.clone()],
        "被拒的请求不应动到既有角色（replace 是整体替换语义，最怕清空）"
    );

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
}

/// 角色列表里有一个不存在 → **整体**拒绝，不能"先写合法的那个"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn one_unknown_role_rejects_the_whole_list() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("atomic_a");
    let ida = create_role_via_api(&app, &token, &a).await;
    let (uid, _) = create_user_with_roles(&app, &token, &["user".to_string()]).await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{uid}"),
            Some(&token),
            Some(json!({
                "username": unique("atomic_user"),
                "email": format!("{}@example.com", unique("atomic_email")),
                "roles": [a.clone(), "不存在的角色"],
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "含未知角色的列表应被整体拒绝: {body}"
    );

    assert_eq!(
        role_names_in_db(uid).await,
        vec!["user".to_string()],
        "列表中合法的那个角色也不该被写入——要么全成要么全不成"
    );

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
}

/// 重复角色去重：`user_roles` 的唯一约束是 `(user_id, role_id)`
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn duplicate_roles_are_collapsed() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("dup_a");
    let ida = create_role_via_api(&app, &token, &a).await;
    let (uid, _) = create_user_with_roles(&app, &token, &[a.clone(), a.clone(), a.clone()]).await;

    assert_eq!(
        role_names_in_db(uid).await,
        vec![a.clone()],
        "重复角色应去重"
    );

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
}

/// 兼容：单数 `role` 字段仍然可用，既有客户端不必改
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn the_legacy_single_role_field_still_assigns_one_role() {
    let app = app().await;
    let token = admin_token(&app).await;

    let a = unique("legacy_a");
    let ida = create_role_via_api(&app, &token, &a).await;
    let username = unique("legacy_user");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "role": a,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "单数 role 字段应仍可用: {body}");

    let uid = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap();
    assert_eq!(
        role_names_in_db(uid).await.len(),
        1,
        "单数字段应只赋一个角色"
    );

    delete_user_via_api(&app, &token, uid).await;
    let _ = delete_role(&app, &token, ida).await;
}

/// 授权下界对**多个**角色同时生效：只持 `user:create` 的角色，
/// 建一个 `roles=[user, admin]` 的账号必须被拒
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn granting_several_roles_at_once_still_hits_the_superset_rule() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    // 只给 user:create —— 能过接口级守卫，但码集远小于 admin
    let (tok, _role_id, _uid) =
        operator_with_codes(&app, &admin_tok, "hr", &[permission::USER_CREATE]).await;

    let username = unique("multi_escalate");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&tok),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                // 第一个角色无权限码、第二个是 admin：
                // 逐个校验就能发现越权，不能只校验最后一个或只看有没有 admin
                "roles": ["user", "admin"],
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "多角色里藏 admin 同样应被拦: {body}"
    );
    assert!(!user_exists(&username).await, "被拒后不应留下半成品用户");
}

/// 契约测试：每个 `/api/admin/*` handler 都必须声明类型化权限码守卫。
///
/// 防止将来新增管理接口时漏接守卫，从而绕过授权体系。守卫是签名里的提取器参数
/// （`_perm: PermUserList`），在**提取阶段**完成校验，因此早于 `Json` 入参解析。
#[test]
fn every_admin_handler_declares_a_permission_guard() {
    let controller_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controller");

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&controller_dir).expect("读取 src/controller 失败") {
        let path = entry.expect("目录项读取失败").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读取 controller 源文件失败");

        // 按 `pub async fn name(...) -> ... {` 切分函数体
        let mut cursor = 0usize;
        while let Some(start) = source[cursor..].find("pub async fn ") {
            let abs_start = cursor + start;
            let body_start = match source[abs_start..].find('{') {
                Some(offset) => abs_start + offset,
                None => break,
            };
            let signature = &source[abs_start..body_start];
            let name: String = signature
                .trim_start_matches("pub async fn ")
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();

            // 路由路径写在函数上方的 `#[utoipa::path(...)]` 属性里，不在签名中
            let attr_start = source[..abs_start]
                .rfind("#[utoipa::path(")
                .unwrap_or(abs_start);
            let attr = &source[attr_start..abs_start];

            // 函数体到下一个 `pub async fn` 或文件末尾
            let rest = &source[body_start..];
            let body = match rest.find("\npub async fn ") {
                Some(offset) => &rest[..offset],
                None => rest,
            };
            cursor = body_start + body.len();

            // 只校验挂在 /api/admin/ 下的 handler
            if !attr.contains("\"/api/admin/") {
                continue;
            }
            checked += 1;

            // 守卫是签名里的类型化提取器参数，在提取阶段完成校验。
            // 参数名允许带或不带 `_` 前缀：v0.5.0 PR-3 起部分 handler 除提取阶段校验外，
            // 还要在函数体里用它做"能否授予他人权限"的包含关系判定，
            // 此时写成 `perm: Perm…`（去掉 `_` 以免 unused 警告）。
            let has_guard = signature.contains("_perm: Perm") || signature.contains("perm: Perm");
            if !has_guard {
                offenders.push(format!(
                    "{}::{name} 未声明类型化权限码守卫（_perm/perm: Perm…）",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }

    assert!(
        checked > 20,
        "应至少校验 20 个 admin handler，实际 {checked}"
    );
    assert!(
        offenders.is_empty(),
        "以下管理接口缺少权限码守卫:\n  {}",
        offenders.join("\n  ")
    );
}

/// **PR-3 核心不变量的回归防线**：凡是"能把权限给别人"的写路径，
/// 都必须在函数体里做授权下界判定（能授予的 ⊆ 自己已持有的）。
///
/// 撤掉 `require_role("admin")` 后，权限码守卫只回答"这个接口能不能调"，
/// 不再回答"能不能把权限给别人"。少了下面这层，包含 `system:user:create`
/// 的角色就能建出 admin 用户、包含 `system:user:update` 的角色就能
/// 重置 admin 口令——**接口级 403 全绿，系统却已完成提权**，
/// 只看接口返回码的测试发现不了，所以这条按源码结构断言。
#[test]
fn every_authority_delegating_handler_checks_the_superset_rule() {
    /// (文件名, handler 名, 必须在函数体里出现的判定调用)
    const REQUIRED: &[(&str, &str, &str)] = &[
        // ── 授予角色：建号 / 改号 / 追加角色 ──
        ("user.rs", "create_user", "ensure_can_grant_roles"),
        ("user.rs", "update_user", "ensure_can_grant_roles"),
        ("role.rs", "assign_user_role", "ensure_can_grant_roles"),
        // ── 作用于既有账号：接管/停用/删除 ──
        // reset_user_password 是其中最直接的一条：拿到 admin 的新口令
        // 就等于登录成 admin，不需要再走"授予角色"。
        ("user.rs", "reset_user_password", "ensure_can_grant_roles"),
        ("user.rs", "delete_user", "ensure_can_grant_roles"),
        ("user.rs", "batch_delete_users", "ensure_can_grant_roles"),
        ("user.rs", "toggle_user_status", "ensure_can_grant_roles"),
        // ── 直接分配权限码：meta 能力，漏掉就能自授全部码 ──
        ("menu.rs", "assign_role_menus", "ensure_covers"),
        // 改写已授权按钮的 permission = 绕过 role_menus 链当场自授
        ("menu.rs", "update_menu", "ensure_covers"),
    ];

    let controller_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controller");
    let mut offenders: Vec<String> = Vec::new();

    for (file, name, needle) in REQUIRED {
        let path = controller_dir.join(file);
        let source = std::fs::read_to_string(&path).expect("读取 controller 源文件失败");

        // 与 every_admin_handler_declares_a_permission_guard 同一套切分逻辑
        let needle_marker = format!("pub async fn {name}(");
        let Some(start) = source.find(&needle_marker) else {
            offenders.push(format!("{file}::{name} 未找到 handler 定义"));
            continue;
        };
        let body_start = source[start..]
            .find('{')
            .map(|o| start + o)
            .expect("handler 应有函数体");
        let rest = &source[body_start..];
        let body = match rest.find("\npub async fn ") {
            Some(o) => &rest[..o],
            None => rest,
        };

        if !body.contains(needle) {
            offenders.push(format!(
                "{file}::{name} 是能授予权限的写路径，但缺少 {needle} 授权下界判定"
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "以下写路径缺少授权下界判定（撤掉角色闸门后即提权）:\n  {}",
        offenders.join("\n  ")
    );
}

/// 角色闸门必须保持删除状态。
///
/// `require_role("admin")` 与权限码守卫曾是 AND 语义：它天然盖住了提权路径，
/// 但也因此让"能进管理区"变成角色名说了算。PR-3 把它连同函数一起删掉，
/// 避免留下一个无人调用的角色闸门，在下一次改动里被误当成"更安全的兜底"接回去。
#[test]
fn the_role_gate_stays_removed() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    /// 去掉 `//` 行注释后再匹配：
    /// 注释里提到 `require_role` 是**有意保留的迁移说明**，不是调用点。
    fn strip_line_comments(src: &str) -> String {
        src.lines()
            .map(|line| match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    let router = strip_line_comments(
        &std::fs::read_to_string(manifest.join("src/router/mod.rs"))
            .expect("读取 src/router/mod.rs 失败"),
    );
    assert!(
        !router.contains("require_role"),
        "路由层不应再挂角色闸门：权限码守卫已是唯一闸门"
    );

    let auth = strip_line_comments(
        &std::fs::read_to_string(manifest.join("src/middleware/auth.rs"))
            .expect("读取 src/middleware/auth.rs 失败"),
    );
    assert!(
        !auth.contains("fn require_role"),
        "require_role 已无调用方，应删除而不是留着备用"
    );
}

// ──────────────────────────────────────────────
// PR-3：角色闸门撤掉后的授权下界
// ──────────────────────────────────────────────

async fn user_id_by_username(username: &str) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&pool().await)
        .await
        .expect("按用户名查 id 失败")
}

// ──────────────────────────────────────────────
// 接口指标聚合（Redis）与审计日志保留
// ──────────────────────────────────────────────

/// 造一个独立的指标收集器，等价于"另一个副本"：各自持有本地缓冲，写同一份 Redis
///
/// flush_interval 设成 60s 且**不启动后台任务**，由用例显式调 `flush`，
/// 这样断言不依赖时间，失败时也不会是"等得不够久"这种含糊原因。
async fn replica_collector() -> MetricsCollector {
    let redis = Arc::new(
        RedisClient::new(&RedisConfig {
            url: test_redis_url(),
        })
        .await
        .expect("连接 Redis 失败"),
    );
    MetricsCollector::new(
        redis,
        MetricsConfig {
            flush_interval_seconds: 60,
            key_ttl_seconds: 600,
            max_buffered_endpoints: 100,
        },
    )
}

fn find_metric<'a>(snapshot: &'a [EndpointMetric], path: &str) -> Option<&'a EndpointMetric> {
    snapshot.iter().find(|m| m.path == path)
}

/// 指标必须按**路由模板**归并，而不是按含真实 ID 的原始路径
///
/// 旧实现记 `req.uri().path()`，于是每个资源 ID 都是独立一条：
/// 基数无界，且每条 `call_count` 恒为 1，监控页看不出这个接口的真实 QPS。
#[tokio::test]
#[ignore]
async fn metrics_group_paths_by_route_template_not_by_resource_id() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 共享 Redis，先清干净再看
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/monitor/metrics/reset",
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重置指标失败: {body}");

    let admin_id = user_id_by_username("admin").await;
    let stranger_id = uuid::Uuid::new_v4();

    // 同一个路由模板，两个不同的资源 ID
    for id in [admin_id, stranger_id] {
        let (status, body) = send(
            &app,
            request(
                "GET",
                &format!("/api/admin/users/{id}/roles"),
                Some(&token),
                None,
            ),
        )
        .await;
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "请求应命中路由（404 表示用户不存在，指标同样应被记录）: {status} {body}"
        );
    }

    // 走真实接口读取：中间件记 → 本地缓冲 → snapshot 合并 → JSON，全程串起来
    let (status, body) = send(
        &app,
        request("GET", "/api/admin/monitor/api-metrics", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "读取指标失败: {body}");
    // 直接断言线上 JSON，而不是反序列化成同一个结构体：
    // 后者会把"字段名写错"这种问题一起糊过去
    let snapshot = body["data"]["metrics"]
        .as_array()
        .expect("响应应含 metrics 数组")
        .clone();

    assert_eq!(
        snapshot
            .iter()
            .find(|m| m["path"] == "/api/admin/users/{user_id}/roles")
            .and_then(|m| m["call_count"].as_u64()),
        Some(2),
        "两次调用应归并到同一个路由模板，实际快照: {snapshot:#?}"
    );

    // method 必须干净：Redis 键带 `metrics:ep:` 前缀，
    // 拆键时忘了剥前缀就会漏成 "metrics:ep:GET"
    assert_eq!(
        snapshot
            .iter()
            .find(|m| m["path"] == "/api/admin/users/{user_id}/roles")
            .and_then(|m| m["method"].as_str()),
        Some("GET"),
        "method 不应带上 Redis 键前缀: {snapshot:#?}"
    );

    // 更强的断言：不能有任何一条按真实 ID 建的记录
    let by_raw_id: Vec<_> = snapshot
        .iter()
        .filter(|m| {
            m["path"]
                .as_str()
                .is_some_and(|p| p.contains(&admin_id.to_string()))
        })
        .collect();
    assert!(
        by_raw_id.is_empty(),
        "指标里不应出现含真实 ID 的路径（否则基数无界）: {by_raw_id:#?}"
    );
}

/// 多个副本的指标必须聚合到同一份计数上
///
/// 这是"多副本不准"的回归：旧实现每个进程一份 HashMap，
/// 监控页上的 QPS 只是本副本的份额。
#[tokio::test]
#[ignore]
async fn metrics_from_several_replicas_aggregate_into_one_view() {
    let probe = replica_collector().await;
    probe.reset().await;

    let replica_a = replica_collector().await;
    let replica_b = replica_collector().await;

    for _ in 0..3 {
        replica_a.record("GET", "/api/admin/monitor/system", 10, false);
    }
    replica_a.record("GET", "/api/admin/monitor/system", 30, true);
    for _ in 0..2 {
        replica_b.record("GET", "/api/admin/monitor/system", 20, false);
    }

    replica_a.flush().await;
    replica_b.flush().await;

    let metric = find_metric(&probe.snapshot().await, "/api/admin/monitor/system")
        .expect("聚合后应能看到该端点")
        .clone();

    assert_eq!(metric.call_count, 6, "两个副本的调用数应相加");
    assert_eq!(metric.error_count, 1, "错误数也应跨副本累加");
    assert_eq!(metric.total_duration_ms, 3 * 10 + 30 + 2 * 20);
    assert_eq!(metric.avg_duration_ms, 100 / 6);
    assert_eq!(metric.min_duration_ms, 10);
    assert_eq!(metric.max_duration_ms, 30);
    assert_eq!(metric.method, "GET", "method 不应带上 Redis 键前缀");
    assert_eq!(metric.path, "/api/admin/monitor/system");

    probe.reset().await;
}

/// `reset` 必须是**跨副本**的
///
/// 旧实现只清本进程的 HashMap，别的副本照旧累加，
/// 于是管理员点完"重置"，监控页的数字立刻又涨回来。
#[tokio::test]
#[ignore]
async fn reset_clears_metrics_for_every_replica() {
    let probe = replica_collector().await;
    probe.reset().await;

    let replica_a = replica_collector().await;
    let replica_b = replica_collector().await;

    // 两个副本都先落 Redis：此刻探针看到的是"两个副本的合计"
    replica_a.record("GET", "/api/admin/users", 5, false);
    replica_a.flush().await;
    replica_b.record("GET", "/api/admin/roles", 5, false);
    replica_b.flush().await;

    assert_eq!(probe.snapshot().await.len(), 2, "前置条件：两个端点都在");

    // 由 B 发起重置，模拟"任意副本收到重置请求"：
    // 删的是 Redis 里的键，A 已落库的那份也必须一起没
    replica_b.reset().await;

    let after = probe.snapshot().await;
    assert!(
        after.is_empty(),
        "重置应清掉所有副本的指标（含 Redis 中已落库的），实际仍有: {after:#?}"
    );
}

/// 尚未 flush 的本地增量也必须立刻可见
///
/// 否则刚发生的调用要等最多一个 flush 间隔才出现在监控页上。
///
/// 注意只能由**记录它的那个实例**来观察：本地缓冲是实例私有的，
/// 别的副本看不到（这正是 flush 要解决的问题）。
#[tokio::test]
#[ignore]
async fn snapshot_includes_deltas_that_have_not_been_flushed_yet() {
    let probe = replica_collector().await;
    probe.reset().await;

    let replica = replica_collector().await;
    replica.record("GET", "/api/admin/dict/items", 12, false);
    // 故意不 flush

    let metric = find_metric(&replica.snapshot().await, "/api/admin/dict/items")
        .expect("未 flush 的增量也应出现在快照里")
        .clone();
    assert_eq!(metric.call_count, 1);
    assert_eq!(metric.total_duration_ms, 12);

    // flush 之后同样的数据仍在（不是"读一次就消失"）
    replica.flush().await;
    let after = find_metric(&replica.snapshot().await, "/api/admin/dict/items")
        .expect("flush 后应仍能从 Redis 读到")
        .clone();
    assert_eq!(after.call_count, 1, "flush 不应重复计数");
    assert_eq!(after.total_duration_ms, 12);

    probe.reset().await;
}

/// 同一端点"已落 Redis + 又有新调用在本地缓冲"时，只能出现**一行**且计数为两者之和
///
/// 这是个真实的合并陷阱：Redis 键带 `metrics:ep:` 前缀，
/// 本地缓冲键不带。若两条直接塞进同一张 map 而不归一化键格式，
/// 它们会变成两行独立记录——监控页上同一个接口出现两次，
/// 每行的 call_count 都只是部分值。
#[tokio::test]
#[ignore]
async fn one_endpoint_stays_one_row_when_it_is_partly_flushed_and_partly_pending() {
    let probe = replica_collector().await;
    probe.reset().await;

    let replica = replica_collector().await;
    // 先记录并 flush：此时计数在 Redis 里（键带前缀）
    for _ in 0..2 {
        replica.record("GET", "/api/admin/monitor/system", 10, false);
    }
    replica.flush().await;
    // 再记录但不 flush：此时计数在本地缓冲里（键不带前缀）
    replica.record("GET", "/api/admin/monitor/system", 40, false);

    let rows: Vec<_> = replica
        .snapshot()
        .await
        .into_iter()
        .filter(|m| m.path == "/api/admin/monitor/system")
        .collect();

    assert_eq!(rows.len(), 1, "同一端点必须合并成一行，实际: {rows:#?}");
    assert_eq!(rows[0].call_count, 3, "Redis 2 次 + 缓冲 1 次 = 3");
    assert_eq!(rows[0].total_duration_ms, 2 * 10 + 40);

    probe.reset().await;
}

/// 插一条指定时间的操作日志，返回其 id
async fn insert_audit_log(created_at: chrono::DateTime<chrono::Utc>) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO audit_logs (action, method, path, created_at) \
         VALUES ('retention_test', 'GET', '/api/retention-test', $1) RETURNING id",
    )
    .bind(created_at)
    .fetch_one(&pool().await)
    .await
    .expect("插入测试用操作日志失败")
}

async fn audit_log_exists(id: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_logs WHERE id = $1)")
        .bind(id)
        .fetch_one(&pool().await)
        .await
        .expect("查询操作日志失败")
}

/// 保留策略只删过期行，不碰仍在保留期内的行
#[tokio::test]
#[ignore]
async fn audit_log_retention_removes_only_expired_rows() {
    let repo = AuditLogRepository::new(pool().await);
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(30);

    let mut expired = Vec::new();
    for _ in 0..3 {
        expired.push(insert_audit_log(now - chrono::Duration::days(40)).await);
    }
    let mut kept = Vec::new();
    for _ in 0..2 {
        kept.push(insert_audit_log(now - chrono::Duration::days(10)).await);
    }

    let deleted = repo.delete_older_than(cutoff, 10, 5).await.unwrap();
    assert_eq!(deleted, 3, "只应删掉 3 条过期日志");

    for id in &expired {
        assert!(!audit_log_exists(*id).await, "过期日志 {id} 应已被删除");
    }
    for id in &kept {
        assert!(audit_log_exists(*id).await, "保留期内的日志 {id} 不该被删");
    }

    // 测试库长期存在，清掉自己造的样本
    sqlx::query("DELETE FROM audit_logs WHERE id = ANY($1)")
        .bind(&kept)
        .execute(&pool().await)
        .await
        .unwrap();
}

/// 删除必须**分批**并受 `max_batches` 约束
///
/// 一次性 `DELETE` 大量行会长时间持锁并撑爆 WAL。
/// 这里用 25 条过期日志、批大小 10、只允许 2 批，
/// 精确验证"删 20 条、剩 5 条"，即批次与上限都真的生效。
#[tokio::test]
#[ignore]
async fn audit_log_retention_deletes_in_bounded_batches() {
    let repo = AuditLogRepository::new(pool().await);
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(1);

    let mut ids = Vec::new();
    for _ in 0..25 {
        ids.push(insert_audit_log(now - chrono::Duration::days(5)).await);
    }

    let deleted = repo.delete_older_than(cutoff, 10, 2).await.unwrap();
    assert_eq!(deleted, 20, "两批 × 每批 10 条，不应超出 max_batches");

    let p = pool().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(count, 5, "应正好剩 5 条待下一轮清理");

    // 放开批次上限后应能删干净
    let deleted = repo.delete_older_than(cutoff, 10, 5).await.unwrap();
    assert_eq!(deleted, 5);
    assert!(!audit_log_exists(ids[0]).await, "清理完后不应有残留");

    sqlx::query("DELETE FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&p)
        .await
        .unwrap();
}

/// 造一个**不是 admin**、只持有指定权限码的操作员，返回 (token, role_id, user_id)
///
/// 这是 PR-3 的核心夹具。撤掉 `require_role("admin")` 之后，"能进管理区"
/// 完全由这些码决定，所以提权面**必须用持有部分码的非 admin 才能测出来**——
/// 拿 admin 当夹具会把所有"应被拒"的断言都测成"当然通过"。
async fn operator_with_codes(
    app: &Router,
    admin_tok: &str,
    prefix: &str,
    codes: &[&str],
) -> (String, uuid::Uuid, uuid::Uuid) {
    let role_name = unique(&format!("{prefix}_role"));
    let role_id = create_role_via_api(app, admin_tok, &role_name).await;

    let mut menu_ids = Vec::new();
    for code in codes {
        menu_ids.push(menu_id_of(code).await);
    }
    let (status, body) = assign_menus(app, admin_tok, role_id, &menu_ids).await;
    assert_eq!(status, StatusCode::OK, "给测试角色授权失败: {body}");

    let username = unique(&format!("{prefix}_user"));
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/users",
            Some(admin_tok),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "role": role_name
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建测试操作员失败: {body}");
    let user_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("创建用户响应里没有 id: {body}"));

    // 管理员建号会置"强制改密"，这里清掉以便夹具返回的是可正常调用接口的令牌。
    // 这些用例关心的是权限码授予，不是改密流程
    let token = activated_token(app, &username, "user1234").await;
    let (_, mine) = send(
        app,
        request("GET", "/api/auth/permissions", Some(&token), None),
    )
    .await;
    let held: Vec<String> =
        serde_json::from_value(mine["data"].clone()).expect("权限码响应格式错误");
    assert_eq!(
        held.len(),
        codes.len(),
        "夹具应恰好只持有指定权限码（admin 全码会让提权断言失去意义），实际 {held:?}"
    );

    (token, role_id, user_id)
}

/// **正向**：持有 `system:user:list` 的非 admin 现在能进管理区。
///
/// PR-3 之前这里必然 403——`require_role("admin")` 拦在权限码之前，
/// 于是"只管查用户"这类自定义角色形同虚设，权限码授权树对它没有意义。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_non_admin_holding_the_code_can_reach_the_interface() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, _role_id, _uid) =
        operator_with_codes(&app, &admin_tok, "readonly_op", &[permission::USER_LIST]).await;

    let (status, body) = send(&app, request("GET", "/api/admin/users", Some(&tok), None)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "持有 system:user:list 的角色应能读用户列表: {body}"
    );

    // 但只有 list 就不能改：权限码仍然逐接口强制
    let admin_id = user_id_by_username("admin").await;
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{admin_id}"),
            Some(&tok),
            Some(json!({
                "username": "admin",
                "email": "admin@example.com",
                "password": "admin123",
                "role": "user"
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "只有 list 码时不应能改用户: {body}"
    );

    // 403 必须发生在校验入参之前，admin 未被误改
    let (status, _) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": "admin", "password": "admin123" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin 账号应未被误改");
}

/// **核心提权防线**：能建号 ≠ 能建管理员。
///
/// 这条正是 PR-2 记下的遗留："持有 `system:user:create` 的管理员可以建出
/// admin 用户——创建用户即等于授予管理员"。闸门撤掉后它从"被角色闸门
/// 顺手盖住"变成"接口级守卫放行"，必须由授权下界显式拦住。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn creating_a_user_with_a_role_you_do_not_hold_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "creator_op",
        &[permission::USER_LIST, permission::USER_CREATE],
    )
    .await;

    let username = unique("escalated_admin");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&tok),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "role": "admin"
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "system:user:create 不得被用来建出 admin: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("admin"),
        "403 文案应指名缺失的权限码，让管理员知道该去勾哪个按钮: {body}"
    );

    // 反向对照：授予无权限码的 user 角色必须放行，
    // 否则这条规则就退化成"一律不许建用户"
    let plain = unique("plain_created");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&tok),
            Some(json!({
                "username": plain,
                "email": format!("{plain}@example.com"),
                "password": "user1234",
                "role": "user"
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "授予无权限码的 user 角色不应被拦（它不授予任何能力）: {body}"
    );
}

/// **最直接的接管路径**：重置高权限账号的口令。
///
/// 比"授予角色"更直接——拿到 admin 的新口令就等于登录成 admin，
/// 不需要再走任何授权链。此前只要有 `system:user:update` 就能做到。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn resetting_a_stronger_account_password_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, _role_id, uid) = operator_with_codes(
        &app,
        &admin_tok,
        "resetter_op",
        &[permission::USER_LIST, permission::USER_UPDATE],
    )
    .await;

    let admin_id = user_id_by_username("admin").await;
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{admin_id}/reset-password"),
            Some(&tok),
            Some(json!({ "password": "hijacked123" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "system:user:update 不得被用来重置 admin 口令: {body}"
    );

    // 反向对照：重置一个权限不高于自己的账号必须放行
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{uid}/reset-password"),
            Some(&tok),
            Some(json!({ "password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重置平级账号口令不应被拦: {body}");
}

/// 追加语义的角色接口曾是一条**独立**的提权路径。
///
/// `POST /users/:id/roles` 是追加而非整体替换，用户表单那道守卫覆盖不到它，
/// 所以 `role_name=admin` 曾可独立生效（PR-2 已记录该语义差异）。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn appending_a_stronger_role_to_a_user_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, _role_id, uid) = operator_with_codes(
        &app,
        &admin_tok,
        "appender_op",
        &[permission::USER_LIST, permission::USER_UPDATE],
    )
    .await;

    // 给自己追加 admin —— 典型的自我提权
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{uid}/roles"),
            Some(&tok),
            // 只给 role_name：与路径重复的 user_id 已是可选，
            // 否则漏传它会得到 422 而不是 403——那就不是在测授权了
            Some(json!({ "role_name": "admin" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "不得给自己追加 admin 角色: {body}"
    );
}

/// `system:menu:grant` 本身就是"把权限码授予角色"的元能力。
///
/// 不设包含关系的话，持有它的角色把全部按钮菜单授予**自己的角色**即可
/// 自授全部权限码——一条与用户/角色完全无关的独立提权路径。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn menu_grant_cannot_self_escalate() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, role_id, _uid) =
        operator_with_codes(&app, &admin_tok, "granter_op", &[permission::MENU_GRANT]).await;

    // 试图把 user:delete 那个按钮授予自己的角色
    let delete_btn = menu_id_of(permission::USER_DELETE).await;
    let (status, body) = assign_menus(&app, &tok, role_id, &[delete_btn]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "menu:grant 不得被用来授予自己没有的权限码: {body}"
    );
    assert!(
        !granted_menu_ids(role_id).await.contains(&delete_btn),
        "被拒后授权必须完全没写入"
    );

    // 授予自己已持有的 menu:grant 是等集，必须放行
    // （否则该角色连维持自己的授权都做不到）
    let grant_btn = menu_id_of(permission::MENU_GRANT).await;
    let (status, body) = assign_menus(&app, &tok, role_id, &[grant_btn]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "授予自己已持有的权限码不应被拦（等集是子集）: {body}"
    );

    // 边界另一侧：把**未持有**的码授予**别的**角色必须放行。
    //
    // 这是"定义权限码"与"持有权限码"两件事的分离点——admin 造一个新码再分发给
    // 各角色是权限码即数据的核心工作流，若一并禁掉，权限码就退化成只能读不能写。
    // 授给别人不会让调用者变强；而那个角色之后若被授给调用者，
    // 会在 `ensure_can_grant_roles` 的包含关系判定处被拦住。
    let other_role = create_role_via_api(&app, &admin_tok, &unique("grantee_role")).await;
    let (status, body) = assign_menus(&app, &tok, other_role, &[delete_btn]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "把未持有的码授予别的角色不应被拦: {body}"
    );
    assert!(
        granted_menu_ids(other_role).await.contains(&delete_btn),
        "授予必须真的写入"
    );
}

/// 改写**已授权菜单**的 `permission` 是绕过 `role_menus` 链自授权限码的旁路。
///
/// 只守 `assign_role_menus` 是不够的：`menus.permission` 本身就是权限码，
/// 把一个已经授予调用者的按钮改成自己没持有的码，
/// "角色→菜单→权限码"这条链会当场在自己身上生效——既不需要 `menu:grant`，
/// 也不需要新建菜单。
///
/// **为什么用一次性临时码，而不是直接拿 `system:user:delete` 做演示：**
/// 真实码已经挂在"删除用户"那行上，`menus.permission` 的唯一索引（迁移 007）
/// 会先于守卫报错；绕开它就得先清空那一行，于是这条用例会顺手把 admin 的
/// `system:user:delete` 摘掉——共享测试库里 admin 平白少一个码，
/// 症状要等到别的用例才爆出来。临时码没这个副作用：
/// 守卫只比对**码的集合关系**，与码值本身无关，拦下 `tmp:priv:xxxxxx`
/// 就等于拦下 `system:user:delete`。
///
/// 同样地，攻击需要**两步**才能绕过唯一索引：先清空临时按钮腾出这个码，
/// 再把它改指到自己已获授权的按钮上。断言因此真的落在守卫上，
/// 而不是被数据库挡在前面。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn rewriting_a_granted_menu_permission_to_an_unheld_code_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "menu_editor",
        &[permission::MENU_LIST, permission::MENU_UPDATE],
    )
    .await;

    let list_btn = menu_id_of(permission::MENU_LIST).await;
    assert!(
        granted_menu_ids(role_id).await.contains(&list_btn),
        "夹具应已把 menu:list 按钮授予自己的角色"
    );

    // admin 造一个带一次性临时码的按钮，**不授予任何角色**——
    // 正好落在"存在于系统、但调用者不持有"这个位置。
    let tmp_code = format!("tmp:priv:{}", &unique("p")[5..]);
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "name": unique("tmp_priv_dir"),
                "type": "directory",
                "path": format!("/{}", unique("tmp")),
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时目录失败: {body}");
    let tmp_dir =
        uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).expect("临时目录响应里没有 id");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "parent_id": tmp_dir,
                "name": "临时私有按钮",
                "type": "button",
                "permission": tmp_code,
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时按钮失败: {body}");
    let tmp_btn =
        uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).expect("临时按钮响应里没有 id");

    // 第一步：清空临时按钮的 permission，腾出这个码。
    // 清空是"移除权限"而非"授予权限"，守卫不拦它。
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{tmp_btn}"),
            Some(&tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空按钮权限码应放行: {body}");

    // 第二步：唯一索引已经腾出位置，此时改指必须由守卫拦住
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{list_btn}"),
            Some(&tok),
            Some(json!({ "permission": tmp_code })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "两步改指（先清空再改写）应被守卫拦住: {body}"
    );

    // 该按钮的 permission 必须原样未被改写：查库确认，而不是只看响应码
    let actual: Option<String> = sqlx::query_scalar("SELECT permission FROM menus WHERE id = $1")
        .bind(list_btn)
        .fetch_one(&pool().await)
        .await
        .expect("读取菜单 permission 失败");
    assert_eq!(
        actual.as_deref(),
        Some(permission::MENU_LIST),
        "被拒后 permission 必须原样未被改写"
    );

    // 该操作员仍不应持有这个临时码
    let (_, mine) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(&tok), None),
    )
    .await;
    let held: Vec<String> = serde_json::from_value(mine["data"].clone()).expect("权限码格式错误");
    assert!(!held.contains(&tmp_code), "提权未生效，实际持有 {held:?}");

    // 清理：临时按钮没授予过任何角色，删掉父目录即级联清掉，不碰任何真实码。
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{tmp_dir}"),
            Some(&admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "删除临时目录失败: {body}");
    assert!(
        !menu_still_exists(tmp_dir).await,
        "临时目录应已从共享测试库清干净"
    );
    assert!(
        !menu_still_exists(tmp_btn).await,
        "临时按钮应随父目录级联删除"
    );
}

// ──────────────────────────────────────────────
// 权限码清空后的恢复路径（v0.7.0）
// ──────────────────────────────────────────────

/// 造一个"带专属权限码的按钮"，并把该按钮授予一个独立角色。
///
/// 授予**独立**角色而不是夹具自己的角色很关键：权限码来自"角色→菜单"，
/// 把按钮授予夹具角色会让夹具自己持有那个码，
/// 于是"不持有该码的操作员"这个前提就不成立了。
///
/// 返回值里的持有者令牌也不能图省事用 admin：种子的"只授权新建行"策略
/// 意味着**新建的码不会自动进 admin**（否则管理员在菜单页撤销的授权
/// 会被下次启动悄悄恢复）。所以 admin 恰恰是那个不持有新码的人。
/// 要模拟"有权清空的人"，得造一个真的在该角色里的用户。
///
/// 顺序有讲究，且被授权下界卡着：**必须先建角色与用户、再授权按钮**。
/// 反过来（先授权再建用户）会失败——`ensure_can_grant_roles` 要求
/// "建号时赋予的角色，其码集 ⊆ 你的码集"，而 admin 恰恰不持有这个新码。
async fn granted_temp_button(
    app: &Router,
    admin_tok: &str,
    code_prefix: &str,
) -> (uuid::Uuid, uuid::Uuid, String, String, uuid::Uuid) {
    // ① 空角色 + 空用户：此刻它不含任何码，建号的下界才过得去
    let holder_role_name = unique("tmp_holder_role");
    let holder_role = create_role_via_api(app, admin_tok, &holder_role_name).await;
    let username = unique("tmp_holder_user");
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/users",
            Some(admin_tok),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "role": holder_role_name
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建持有者失败: {body}");
    let holder_uid = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();
    let holder_tok = activated_token(app, &username, "user1234").await;

    // ② 再建带专属码的按钮
    let dir_name = unique("tmp_restore_dir");
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/menus",
            Some(admin_tok),
            Some(json!({
                "name": dir_name,
                "type": "directory",
                "path": format!("/{dir_name}"),
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时目录失败: {body}");
    let dir_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let code = format!("{code_prefix}:priv:{}", &unique("p")[5..]);
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/menus",
            Some(admin_tok),
            Some(json!({
                "parent_id": dir_id,
                "name": "待清空的按钮",
                "type": "button",
                "permission": code,
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时按钮失败: {body}");
    let btn_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    // ③ 最后授权：该角色不是 admin 自己的角色，`assign_role_menus` 的自授守卫不介入。
    // 顺带给他 `system:menu:update`——清空/恢复本身就要这个码，
    // 少了它下面测的是 403 "缺少权限码" 而不是清空守卫。
    let menu_update_btn = menu_id_of(axum_api::model::permission::MENU_UPDATE).await;
    let (status, body) =
        assign_menus(app, admin_tok, holder_role, &[menu_update_btn, btn_id]).await;
    assert_eq!(status, StatusCode::OK, "授予临时角色失败: {body}");

    // 夹具自检：持有者必须真的经"角色→菜单"拿到这个码，否则下面全是空测
    let (_, body) = send(
        app,
        request("GET", "/api/auth/permissions", Some(&holder_tok), None),
    )
    .await;
    let held: Vec<String> = serde_json::from_value(body["data"].clone()).unwrap();
    assert!(
        held.contains(&code),
        "夹具未生效：持有者应持有 {code}，实得 {held:?}"
    );
    assert!(
        held.iter()
            .any(|c| c == axum_api::model::permission::MENU_UPDATE),
        "夹具未生效：持有者还应能改菜单，实得 {held:?}"
    );

    (dir_id, btn_id, code, holder_tok, holder_uid)
}

async fn permission_of(menu_id: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT permission FROM menus WHERE id = $1")
        .bind(menu_id)
        .fetch_one(&pool().await)
        .await
        .expect("读取菜单 permission 失败")
}

async fn restore_slot(menu_id: uuid::Uuid) -> (Option<String>, Option<uuid::Uuid>) {
    sqlx::query_as("SELECT prev_permission, prev_permission_cleared_by FROM menus WHERE id = $1")
        .bind(menu_id)
        .fetch_one(&pool().await)
        .await
        .expect("读取恢复槽位失败")
}

/// 清理临时菜单目录
///
/// `delete_menu` 自 v0.8.0 第 1 项起对"已被授予角色的码"设了守卫，而 admin
/// 造这些一次性临时码时**并没有持有**它们，于是 admin 不能直接删掉——
/// 403 是正确行为，不是缺陷。
///
/// 恢复路径是产品设计的一部分：先撤销该菜单的授权（需 `system:menu:grant`），
/// 菜单变成"没人依赖"后删除即放行。
///
/// 这里直连 SQL 删 `role_menus` 而不是走 `PUT /roles/:id/menus`：
/// 后者是**全量替换**，会顺手清掉那些临时角色身上别的授权。
async fn cleanup_temp_menu_dir(app: &Router, admin_tok: &str, dir_id: uuid::Uuid) {
    sqlx::query(
        "DELETE FROM role_menus WHERE menu_id IN (
             WITH RECURSIVE subtree AS (
                 SELECT id FROM menus WHERE id = $1
                 UNION ALL
                 SELECT m.id FROM menus m JOIN subtree s ON m.parent_id = s.id
             )
             SELECT id FROM subtree
         )",
    )
    .bind(dir_id)
    .execute(&pool().await)
    .await
    .expect("清理临时授权失败");

    let (status, body) = send(
        app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理临时目录失败: {body}");
}

/// 核心闭环：清空 → 记录凭据 → **死路演示** → 恢复 → 凭据一次性作废
///
/// "死路演示"那一步是本 PR 存在的理由：清空后没有任何角色再持有该码，
/// 于是 `update_menu` 的守卫会把**写回**也一并拦死。
/// 修复前这里无路可走，只能去新建一个孤儿按钮。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn clearing_a_permission_code_can_be_restored_by_the_clearing_user() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, holder_tok, holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:restore").await;

    // 前提：该码确实已授予某个角色（否则清空不改变任何人的权限，守卫不会介入）
    assert!(grant_count_for_menu(btn_id).await > 0, "夹具应已授予该按钮");

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&holder_tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "持有者清空自己持有的码应放行: {body}"
    );
    assert_eq!(permission_of(btn_id).await, None, "码应已从按钮上清空");

    let (prev, cleared_by) = restore_slot(btn_id).await;
    assert_eq!(prev.as_deref(), Some(code.as_str()), "应留下可恢复的码");
    assert_eq!(cleared_by, Some(holder_uid), "应记下清空者");

    // 死路演示：清空后没人再持有该码，update_menu 会把写回也拦死
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&holder_tok),
            Some(json!({ "permission": code })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "这正是需要恢复接口的原因：写回被自己的守卫拦死: {body}"
    );

    // 恢复：只有清空者本人能调，且不要求当前持有该码
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/menus/{btn_id}/restore-permission"),
            Some(&holder_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空者本人应能恢复: {body}");
    assert_eq!(
        body["data"]["permission"],
        json!(code),
        "响应应带回恢复后的码"
    );
    assert_eq!(permission_of(btn_id).await.as_deref(), Some(code.as_str()));

    // 凭据一次性作废：不能用同一个槽位反复"清空→恢复"
    let (prev, cleared_by) = restore_slot(btn_id).await;
    assert_eq!(prev, None, "恢复后凭据应作废");
    assert_eq!(cleared_by, None, "恢复后不应再记着清空者");
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/menus/{btn_id}/restore-permission"),
            Some(&holder_tok),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "凭据用过后不应能重复恢复: {body}"
    );

    cleanup_temp_menu_dir(&app, &admin_tok, dir_id).await;
    assert!(!menu_still_exists(btn_id).await, "临时按钮应已清干净");
}

/// 清空一个**别人正在用**的码 = 跨角色撤权，必须持有该码
///
/// 修复前 `.filter(|p| !p.is_empty())` 让清空整个绕过守卫，
/// 持 `system:menu:update` 的角色可以把别的角色已持有的按钮的码清掉，
/// 绕过 `system:menu:grant`。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn clearing_a_code_others_rely_on_requires_holding_it() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, _holder_tok, _holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:clear").await;

    // 操作员只持 menu:update，不持那个一次性码
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "clear_denied",
        &[permission::MENU_LIST, permission::MENU_UPDATE],
    )
    .await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "清空别人依赖的码应被拒: {body}"
    );
    assert_eq!(
        permission_of(btn_id).await.as_deref(),
        Some(code.as_str()),
        "被拒后码必须原样未被清空"
    );

    cleanup_temp_menu_dir(&app, &admin_tok, dir_id).await;
}

/// 清空一个**没授予任何角色**的码不改变任何人的权限，应放行
///
/// 这是守卫的另一半：如果连这种无害操作也拦，"整理菜单结构"
/// 就会全线报错。既有测试 `rewriting_a_granted_menu_permission_to_an_unheld_code_is_denied`
/// 的第一步也依赖这一点。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn clearing_a_code_no_role_relies_on_is_allowed() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let dir_name = unique("tmp_harmless_dir");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "name": dir_name,
                "type": "directory",
                "path": format!("/{dir_name}"),
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时目录失败: {body}");
    let dir_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let code = format!("tmp:harmless:{}", &unique("p")[5..]);
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "parent_id": dir_id,
                "name": "无人用的按钮",
                "type": "button",
                "permission": code,
                "sort_order": 99
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建临时按钮失败: {body}");
    let btn_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        grant_count_for_menu(btn_id).await,
        0,
        "夹具前提：该按钮未授予任何角色"
    );

    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "clear_harmless",
        &[permission::MENU_LIST, permission::MENU_UPDATE],
    )
    .await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "清空无人依赖的码属整理菜单，应放行: {body}"
    );
    assert_eq!(permission_of(btn_id).await, None);

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(&admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理临时目录失败: {body}");
}

/// 恢复是"撤销我自己的误操作"，不是"接管别人的清空"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn only_the_clearing_user_can_restore_a_permission_code() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, holder_tok, _holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:owner").await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&holder_tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空应放行: {body}");

    // 另一个同样持 menu:update 的操作员来恢复 —— 不是他清的
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "restore_other",
        &[permission::MENU_LIST, permission::MENU_UPDATE],
    )
    .await;
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/menus/{btn_id}/restore-permission"),
            Some(&tok),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "非清空者不应能恢复: {body}"
    );
    assert!(
        body["message"].as_str().unwrap().contains(&code),
        "403/400 文案应点名是哪个码: {body}"
    );
    assert_eq!(permission_of(btn_id).await, None, "被拒后码不应被写回");

    // 清空者本人仍然能恢复
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/menus/{btn_id}/restore-permission"),
            Some(&holder_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空者本人应能恢复: {body}");
    assert_eq!(permission_of(btn_id).await.as_deref(), Some(code.as_str()));

    cleanup_temp_menu_dir(&app, &admin_tok, dir_id).await;
}

/// 菜单树要告诉前端"这个按钮的码可以恢复"，否则恢复入口无从发现
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn the_menu_tree_reports_a_restorable_permission_code() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, holder_tok, _holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:tree").await;

    let find_node = |nodes: &Value, id: uuid::Uuid| -> Option<Value> {
        fn walk(nodes: &Value, id: uuid::Uuid) -> Option<Value> {
            nodes.as_array()?.iter().find_map(|n| {
                let matched = n["id"].as_str() == Some(&id.to_string());
                if matched {
                    return Some(n.clone());
                }
                walk(&n["children"], id)
            })
        }
        walk(nodes, id)
    };

    // 未清空时不该报可恢复
    let (_, body) = send(
        &app,
        request("GET", "/api/admin/menus", Some(&admin_tok), None),
    )
    .await;
    let node = find_node(&body["data"], btn_id).expect("菜单树里应有该按钮");
    assert!(
        node["restorable_permission"].is_null(),
        "未清空时不该有可恢复码: {node}"
    );

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&holder_tok),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空应放行: {body}");

    let (_, body) = send(
        &app,
        request("GET", "/api/admin/menus", Some(&admin_tok), None),
    )
    .await;
    let node = find_node(&body["data"], btn_id).expect("菜单树里应有该按钮");
    assert_eq!(
        node["restorable_permission"],
        json!(code),
        "清空后应报出可恢复的码: {node}"
    );
    // 刻意不暴露"谁清的"：界面只需知道能不能恢复，鉴权在服务端
    assert!(
        node.get("prev_permission_cleared_by").is_none(),
        "不应把清空者暴露给前端: {node}"
    );

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(&admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理临时目录失败: {body}");
}

/// 正向对照：持有全部权限码的 admin 仍然能建出 admin 用户。
///
/// 授权下界是"包含关系"而非"角色名白名单"，所以这条必须通——
/// 否则规则就退化成"谁都不许管理管理员"，等于把系统锁死。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn admin_can_still_create_another_admin() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let username = unique("second_admin");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&admin_tok),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "admin123",
                "role": "admin"
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin 持全部码，应仍能授予 admin 角色: {body}"
    );
    assert!(
        body["data"]["roles"]
            .as_array()
            .map(|a| a.iter().any(|v| v == "admin"))
            .unwrap_or(false),
        "响应应回显 admin 角色: {body}"
    );

    // 必须清理掉这个第二管理员。
    //
    // 测试库是**长期存在**的共享库，而 `last_admin_cannot_be_demoted_or_deleted`
    // 断言的前提是"库里只有一名管理员"（它把当前登录的 admin 自己降级，
    // 期望被"不能移除最后一名管理员"拦下）。留下第二名管理员会让那个前提
    // 不成立，降级就会真的成功，随后**所有**用 admin 令牌的用例集体 403——
    // 症状出现在别的测试上，极难定位。
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/users/{}", user_id_by_username(&username).await),
            Some(&admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理第二个管理员失败: {body}");
}

// ===== v0.8.0 第 1 项：菜单删除的授权下界 =====

/// 删除一个**别的角色正依赖**的按钮，必须持有该码
///
/// v0.7.0 修好了 `update_menu` 清空已授权按钮的码（要求持该码），
/// 但 `delete_menu` 完全没检查——而两者的**效果等价**：
/// 码都会从目标角色身上消失。区别只是 delete 连按钮行都没了。
///
/// 实测（修复前）：deleter 角色只持 `menu:list` + `menu:delete`，
/// 既不持那个一次性码，也没有 `system:menu:grant`，
/// 却能把别的角色依赖的码整个剥掉，完成一次绕过 `menu:grant` 的跨角色撤权。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_granted_button_others_rely_on_requires_holding_it() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, holder_tok, _holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:del").await;

    // 操作员只持 menu:delete，不持那个一次性码，也没有 menu:grant
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "delete_denied",
        &[permission::MENU_LIST, permission::MENU_DELETE],
    )
    .await;

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&tok),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "删除别人依赖的码应被拒: {body}"
    );

    // 被拒后两件事都必须成立：按钮还在，且持有者的码没被剥掉
    assert_eq!(
        permission_of(btn_id).await.as_deref(),
        Some(code.as_str()),
        "被拒后按钮必须原样存在、码未被清掉"
    );
    let (_, mine) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(&holder_tok), None),
    )
    .await;
    let held: Vec<String> =
        serde_json::from_value(mine["data"].clone()).expect("权限码响应格式错误");
    assert!(
        held.contains(&code),
        "持有者必须仍然持有该码，实际持有: {held:?}"
    );

    // 守卫不能把菜单永久锁死：admin 造了这个码但没持有它，
    // 因此 admin 也无法**直接**删除。出路是先撤销该菜单的授权（需 `menu:grant`），
    // 菜单变成"没人依赖"后删除即放行。这条恢复路径是守卫成立的前提，必须验。
    let holder_role = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT role_id FROM role_menus WHERE menu_id = $1 LIMIT 1",
    )
    .bind(btn_id)
    .fetch_one(&pool().await)
    .await
    .expect("查询持有该按钮的角色失败");

    let (status, body) = assign_menus(&app, &admin_tok, holder_role, &[]).await;
    assert_eq!(status, StatusCode::OK, "撤销对该按钮的授权失败: {body}");

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(&admin_tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "撤销授权后应可删除: {body}");
}

/// 删除一个**没授予任何角色**的按钮不改变任何人的权限，应放行
///
/// 这是守卫的另一半，与 `clearing_a_code_no_role_relies_on_is_allowed` 同理：
/// 连这种无害操作也拦的话，"整理菜单结构"会全线报错。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_button_no_role_relies_on_is_allowed() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let code = format!("tmp:ungranted:{}", &unique("p")[5..]);

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "name": unique("tmp_ungranted_btn"),
                "type": "button",
                "permission": code
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建未被授予的按钮失败: {body}");
    let btn_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "delete_allowed",
        &[permission::MENU_LIST, permission::MENU_DELETE],
    )
    .await;

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{btn_id}"),
            Some(&tok),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "删除没人依赖的按钮应放行: {body}");
}

/// 声明一个**已被占用**的权限码应报冲突，而不是服务器内部错误
///
/// 修复前实测返回 500 "服务器内部错误"：迁移 `007` 的部分唯一索引挡住了它，
/// 但入参错误被当成服务端故障，污染错误监控，管理员也看不懂。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn declaring_an_already_used_permission_code_is_a_conflict() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "name": unique("dup_code_btn"),
                "type": "button",
                "permission": permission::USER_DELETE
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "声明已被占用的码应报冲突，实际: {body}"
    );

    // 原有那个按钮必须完好无损
    assert_eq!(
        permission_of(menu_id_of(permission::USER_DELETE).await)
            .await
            .as_deref(),
        Some(permission::USER_DELETE),
        "冲突不应改动既有按钮"
    );
}

/// 删除**父级目录**同样要拦住——删除是级联的
///
/// `menus.parent_id` 声明了 `ON DELETE CASCADE`，所以删一个目录会连带删掉
/// 整棵子树。若守卫只看目标节点自身的 `permission`，那么
/// "删承载码的按钮"被拦住了，"删它的父目录"却能绕过去——两条路效果完全一样。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_directory_with_a_granted_button_below_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (dir_id, btn_id, code, holder_tok, _holder_uid) =
        granted_temp_button(&app, &admin_tok, "tmp:delcascade").await;

    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "cascade_denied",
        &[permission::MENU_LIST, permission::MENU_DELETE],
    )
    .await;

    // 目标节点是目录，本身不携带任何码
    assert_eq!(
        permission_of(dir_id).await,
        None,
        "夹具前提：目录自身不应携带权限码"
    );

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/menus/{dir_id}"),
            Some(&tok),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "删除子树里有别人依赖的码的目录应被拒: {body}"
    );

    assert_eq!(
        permission_of(btn_id).await.as_deref(),
        Some(code.as_str()),
        "被拒后子树里的按钮必须完好"
    );
    let (_, mine) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(&holder_tok), None),
    )
    .await;
    let held: Vec<String> =
        serde_json::from_value(mine["data"].clone()).expect("权限码响应格式错误");
    assert!(
        held.contains(&code),
        "持有者必须仍然持有该码，实际持有: {held:?}"
    );

    cleanup_temp_menu_dir(&app, &admin_tok, dir_id).await;
}

/// 非法的菜单类型应报 400，而不是服务器内部错误
///
/// `menus_type_check` 只允许 `menu` / `button` / `directory`。
/// 修复前这个约束冲突会冒成 500「服务器内部错误」——入参问题被当成
/// 服务端故障，既污染错误监控，管理员也不知道该改成什么。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_invalid_menu_type_is_a_bad_request() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&admin_tok),
            Some(json!({
                "name": unique("bad_type"),
                "type": "page",
                "path": format!("/{}", unique("badtype"))
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "非法菜单类型应报 400，实际: {body}"
    );
    let msg = body["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("menu") && msg.contains("button") && msg.contains("directory"),
        "报错要告诉管理员合法取值是什么，实际: {msg}"
    );
}

// ──────────────────────────────────────────────
// v0.9.0：`assign_user_role` 的目标用户下界 + 会话吊销 + 404
// ──────────────────────────────────────────────

/// **洞 A**：只查"授予什么角色"，不查"授予给谁"。
///
/// 修复前，一个只持 `system:user:update` 的角色能给一个纯 admin 账号
/// 追加角色（实测 HTTP 200，角色真的从 `["admin"]` 变成 `["admin","user"]`），
/// 而 `update_user` / `delete_user` / `batch_delete_users` 三处都查目标用户。
/// 这是同一条边界在第四个入口上的缺口。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn appending_a_role_to_a_stronger_account_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "target_guard_op",
        &[permission::USER_LIST, permission::USER_UPDATE],
    )
    .await;

    // 目标：持一个上靠强越过操作员的自定义角色。
    //
    // 不直接给目标塞 `admin` 角色：那会在共享测试库里留下第二个
    // 管理员，使 `last_admin_cannot_be_demoted_or_deleted` 失效（`ensure_not_last_admin`
    // 只拒绝“降级最后一个管理员”，留下第二个就合法了）。
    // 这个失败会连带把后面 19 条用 admin 令牌的用例全部打上 403。
    // ——使用自定义强角色同样能验证目标下界，且不留脏数据。
    let strong_role = unique("strong_role");
    let strong_role_id = create_role_via_api(&app, &admin_tok, &strong_role).await;
    let (status, body) = assign_menus(
        &app,
        &admin_tok,
        strong_role_id,
        // 这个码操作员不持有
        &[menu_id_of(permission::ROLE_DELETE).await],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "给强角色授权失败: {body}");

    let strong_name = unique("strong_user");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&admin_tok),
            Some(json!({
                "username": strong_name,
                "email": format!("{strong_name}@example.com"),
                "password": "user1234",
                "roles": [strong_role.clone()]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建高权限目标账号失败: {body}");
    let strong_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("创建用户响应里没有 id: {body}"));

    let before = role_names_in_db(strong_id).await;
    assert_eq!(
        before,
        vec![strong_role.clone()],
        "目标账号应只持强角色，实际 {before:?}"
    );

    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{strong_id}/roles"),
            Some(&tok),
            Some(json!({ "role_name": "user" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "不得给权限高于自己的账号追加角色: {body}"
    );

    // 判据落在数据变化上，不落在状态码上——200 也可能什么都不写
    let after = role_names_in_db(strong_id).await;
    assert_eq!(
        after, before,
        "被拒后目标账号的角色集合必须原样不变，实际 {after:?}"
    );

    // 清理：共享测试库里的每个用例都应身后无残留
    delete_user_via_api(&app, &admin_tok, strong_id).await;
    delete_role(&app, &admin_tok, strong_role_id).await;
}

/// **洞 A 的镜像**：同一端点该拦的拦住了，就不该把合法路径一起拦掉。
///
/// 弱操作员给自己追加一个自己已持有的弱角色，是合法的自我调整，
/// 不是越权。修复前后的差别只该落在"目标权限高于自己"这一侧。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn appending_a_weaker_role_to_yourself_is_still_allowed() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    // 同时持 user:list 与 dict:list，追加 user 角色后两条都还在
    let (tok, _role_id, uid) = operator_with_codes(
        &app,
        &admin_tok,
        "self_append_op",
        &[
            permission::USER_LIST,
            permission::USER_UPDATE,
            permission::DICT_LIST,
        ],
    )
    .await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{uid}/roles"),
            Some(&tok),
            Some(json!({ "role_name": "user" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "给自己追加一个自己已持有的弱角色应放行: {body}"
    );

    let roles = role_names_in_db(uid).await;
    assert!(
        roles.contains(&"user".to_string()),
        "追加应真的落库，实际 {roles:?}"
    );
    assert!(
        roles.len() >= 2,
        "原有角色不应被追加语义覆盖掉，实际 {roles:?}"
    );
}

/// **洞 B**：改了角色却不吊销会话，新权限要等目标用户自己重新登录才生效。
///
/// `update_user` 在角色集合变化时显式吊销存量会话，注释写明
/// "旧令牌不得继续携带旧角色"。本端点改的是同一份数据却缺这一步，
/// 实测：追加含 `system:dict:list` 的角色后，目标用户的**原令牌**仍是 403，
/// 重新登录才变 200。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn appending_a_role_takes_effect_without_waiting_for_a_relogin() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    // 造一个恰好含 DICT_LIST 的角色
    let grant_role = unique("grant_role");
    let grant_role_id = create_role_via_api(&app, &admin_tok, &grant_role).await;
    let (status, body) = assign_menus(
        &app,
        &admin_tok,
        grant_role_id,
        &[menu_id_of(permission::DICT_LIST).await],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "给新角色授权失败: {body}");

    // 目标：只持内置 user 角色，因此打 dict 列表必然 403
    let target_name = unique("lazy_user");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&admin_tok),
            Some(json!({
                "username": target_name,
                "email": format!("{target_name}@example.com"),
                "password": "user1234",
                "roles": ["user"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建目标用户失败: {body}");
    let target_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("创建用户响应里没有 id: {body}"));

    let stale_token = activated_token(&app, &target_name, "user1234").await;
    let (before, _) = send(
        &app,
        request("GET", "/api/admin/dict/types", Some(&stale_token), None),
    )
    .await;
    assert_eq!(
        before,
        StatusCode::FORBIDDEN,
        "授权前目标用户不该能读字典类型"
    );

    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{target_id}/roles"),
            Some(&admin_tok),
            Some(json!({ "role_name": grant_role.clone() })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "追加角色失败: {body}");

    // 关键：仍用**原来那个令牌**（不重新登录）。
    // 修复前它仍然"有效但缺码"（403），新权限要等重新登录才生效；
    // 修复后会话被吊销，它作为**已失效令牌**被拒（401）。
    // 判别点就是这个 403 -> 401：不是让它"用旧令牌拿到新权限"，
    // 而是旧令牌必须立刻作废、由重新登录发一个带新角色的令牌。
    let (after, body) = send(
        &app,
        request("GET", "/api/admin/dict/types", Some(&stale_token), None),
    )
    .await;
    assert_eq!(
        after,
        StatusCode::UNAUTHORIZED,
        "追加角色后旧令牌应立刻失效（会话已吊销）: {body}"
    );

    let fresh = activated_token(&app, &target_name, "user1234").await;
    let (relogin, body) = send(
        &app,
        request("GET", "/api/admin/dict/types", Some(&fresh), None),
    )
    .await;
    assert_eq!(relogin, StatusCode::OK, "重新登录后当然也该可用: {body}");
}

/// **洞 B 的配套**：重复追加同一角色是幂等的，不该把一次无操作变成强制登出。
///
/// `assign_role_to_user` 用 `ON CONFLICT DO NOTHING`，第二次调用什么都没写。
/// 若照样吊销会话，运维点两下保存就把对方踢下线了——那比不吊销更糟。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn re_applying_the_same_role_does_not_kill_the_target_session() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let grant_role = unique("idem_role");
    let grant_role_id = create_role_via_api(&app, &admin_tok, &grant_role).await;
    let (status, body) = assign_menus(
        &app,
        &admin_tok,
        grant_role_id,
        &[menu_id_of(permission::DICT_LIST).await],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "给新角色授权失败: {body}");

    let target_name = unique("idem_user");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&admin_tok),
            Some(json!({
                "username": target_name,
                "email": format!("{target_name}@example.com"),
                "password": "user1234",
                "roles": ["user"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建目标用户失败: {body}");
    let target_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("创建用户响应里没有 id: {body}"));

    // 第一次追加：真的改了角色，应该吊销
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{target_id}/roles"),
            Some(&admin_tok),
            Some(json!({ "role_name": grant_role.clone() })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "首次追加失败: {body}");

    // 重新登录拿到一个"该角色已生效"的令牌
    let live = activated_token(&app, &target_name, "user1234").await;
    let (ok, body) = send(
        &app,
        request("GET", "/api/admin/dict/types", Some(&live), None),
    )
    .await;
    assert_eq!(ok, StatusCode::OK, "首次追加后应可用: {body}");

    // 第二次追加同一个角色：什么都没写，会话必须留着
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{target_id}/roles"),
            Some(&admin_tok),
            Some(json!({ "role_name": grant_role.clone() })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重复追加应返回 200: {body}");

    let (still, body) = send(
        &app,
        request("GET", "/api/admin/dict/types", Some(&live), None),
    )
    .await;
    assert_eq!(
        still,
        StatusCode::OK,
        "重复追加是幂等操作，不该吊销目标用户的会话: {body}"
    );
}

/// **第三个洞**：给不存在的用户追加角色返回 500，而不是 404。
///
/// `user_roles.user_id` 有外键指向 `users(id)`，用户不存在时外键违例
/// 直接冒成 500「服务器内部错误」——与 v0.8.0 修的
/// 「声明已占用的权限码冒成 500」同源：入参错误被当成服务端故障。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn appending_a_role_to_an_unknown_user_is_not_found() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users/00000000-0000-0000-0000-000000000000/roles",
            Some(&admin_tok),
            Some(json!({ "role_name": "user" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "目标用户不存在应报 404，实际: {body}"
    );
}

// ──────────────────────────────────────────────
// v0.9.0（探针发现的第四个洞）：删角色的授权天花板
// ──────────────────────────────────────────────

/// **洞**：只持 `system:role:delete` 的角色可以删掉一个承载 `system:log:list`
/// 的角色——那个码他自己并不持有。
///
/// 删除角色 = 把这个角色承载的权限码从所有持有者身上撤走，与
/// `PUT /roles/:id/menus`（给角色授权）是同一件事的两面。v0.7.0 给授权那条路
/// 装了天花板，删除这条路当时没管，于是"只能授予自己已持有的权限"这条不变量
/// 在删除上是失效的。
///
/// 是 `e2e/probe-write-guards.mjs` 首次运行时自动报出来的：
/// 写入口清单从 OpenAPI 自动发现，未登记的入口直接报"未覆盖"。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_role_that_carries_permissions_you_lack_is_denied() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    // 目标角色：承载一个操作员拿不到的码
    let strong_name = unique("delrole_strong");
    let strong_id = create_role_via_api(&app, &admin_tok, &strong_name).await;
    let (status, body) = assign_menus(
        &app,
        &admin_tok,
        strong_id,
        &[menu_id_of(permission::LOG_LIST).await],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "给目标角色授权失败: {body}");

    // 操作员：只有 role:delete
    let (tok, _role_id, _uid) =
        operator_with_codes(&app, &admin_tok, "delrole_op", &[permission::ROLE_DELETE]).await;

    let (status, body) = delete_role(&app, &tok, strong_id).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "不得删除承载自己未持有权限码的角色: {body}"
    );
    // 报错要让人看懂缺的是哪个码，否则管理员无从判断该找谁授权
    assert!(
        String::from(body["message"].as_str().unwrap_or("")).contains(permission::LOG_LIST),
        "报错文案应点明缺失的权限码，实际: {body}"
    );

    // 判据落在数据上：被拒之后角色必须还在
    assert!(role_still_exists(strong_id).await, "被拒后角色必须原样保留");

    delete_role(&app, &admin_tok, strong_id).await;
}

/// **洞的镜像**：该拦的拦住之后，合法路径不能被一起堵掉。
///
/// 调用方自己就持有该角色全部权限码时，删它是合法的日常运维动作。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_role_whose_permissions_you_cover_is_allowed() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let target_name = unique("delrole_cover");
    let target_id = create_role_via_api(&app, &admin_tok, &target_name).await;
    let (status, body) = assign_menus(
        &app,
        &admin_tok,
        target_id,
        &[menu_id_of(permission::LOG_LIST).await],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "给目标角色授权失败: {body}");

    // 操作员同时持 role:delete 与 log:list —— 覆盖目标角色的全部码
    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "delrole_cover_op",
        &[permission::ROLE_DELETE, permission::LOG_LIST],
    )
    .await;

    let (status, body) = delete_role(&app, &tok, target_id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "自己覆盖目标角色全部权限码时应当放行: {body}"
    );
    assert!(!role_still_exists(target_id).await, "角色应已被删除");
}

/// 天花板不该误伤**无码角色**：没挂任何权限码的角色删掉不改变任何人的权限。
///
/// 与 v0.8.0 `delete_menu` 的设计同源——那时特意让"没人依赖的码"可以正常删除，
/// 否则"整理角色结构"这类无害操作会全线报错。删角色这里必须保持同一态度。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_role_that_carries_no_permission_is_allowed() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;

    let empty_name = unique("delrole_empty");
    let empty_id = create_role_via_api(&app, &admin_tok, &empty_name).await;
    // 刻意不授权任何菜单

    let (tok, _role_id, _uid) = operator_with_codes(
        &app,
        &admin_tok,
        "delrole_empty_op",
        &[permission::ROLE_DELETE],
    )
    .await;

    let (status, body) = delete_role(&app, &tok, empty_id).await;
    assert_eq!(status, StatusCode::OK, "无码角色不应被天花板拦下: {body}");
    assert!(!role_still_exists(empty_id).await, "角色应已被删除");
}

// ──────────────────────────────────────────────
// v0.10.0：把"假筛选"变成真筛选
//
// 判据一律落在**返回条数**上，不是"参数发出去了"。
// 这一族问题的特点是**从不报错**：参数被前端老老实实发出去，
// 后端安静地当它不存在，界面表现为"搜索没反应"。
// ──────────────────────────────────────────────

/// 建一个用户名可控的账号并清理
async fn mkuser(app: &Router, token: &str, username: &str) -> uuid::Uuid {
    let (status, body) = create_user_via_api(app, token, username, "user").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "创建测试用户 {username} 失败: {body}"
    );
    body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("响应里没有可解析的用户 id: {body}"))
}

async fn query_users(app: &Router, token: &str, query: &str) -> (StatusCode, Value) {
    send(
        app,
        request(
            "GET",
            &format!("/api/admin/users?{query}"),
            Some(token),
            None,
        ),
    )
    .await
}

/// 用户列表按关键字过滤：命中用户名，且每一行都真的含这个关键字
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn user_list_filters_by_username() {
    let app = app().await;
    let token = admin_token(&app).await;

    let needle = unique("kwuser");
    let uid = mkuser(&app, &token, &needle).await;

    let (status, body) = query_users(&app, &token, &format!("keyword={needle}")).await;
    assert_eq!(status, StatusCode::OK, "按关键字查询失败: {body}");

    let items = body["data"]["items"].as_array().expect("items 应为数组");
    assert!(
        !items.is_empty(),
        "命中关键字却返回空列表: keyword={needle}"
    );
    for item in items {
        let name = item["username"].as_str().unwrap_or_default();
        let email = item["email"].as_str().unwrap_or_default();
        assert!(
            name.contains(&needle) || email.contains(&needle),
            "返回了不匹配的行: {name} / {email}"
        );
    }
    // total 必须同步收敛，否则分页错乱（列表 1 条但 total 500）
    assert_eq!(
        body["data"]["total"].as_i64(),
        Some(items.len() as i64),
        "total 与 items 长度不一致: {body}"
    );

    delete_user_via_api(&app, &token, uid).await;
}

/// 关键字也匹配邮箱——搜索框的语义是"找人"，人可能只记得邮箱
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn user_list_filters_by_email() {
    let app = app().await;
    let token = admin_token(&app).await;

    let uid = mkuser(&app, &token, &unique("kwe")).await;
    let needle = unique("mailonly");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{uid}"),
            Some(&token),
            Some(json!({
                "username": format!("kwe_{}", &needle[..8]),
                "email": format!("{needle}@example.com"),
                "roles": ["user"],
                "is_active": true,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改邮箱失败: {body}");

    // 关键字只出现在邮箱里，用户名里没有
    let (status, body) = query_users(&app, &token, &format!("keyword={needle}")).await;
    assert_eq!(status, StatusCode::OK, "按邮箱关键字查询失败: {body}");
    let items = body["data"]["items"].as_array().expect("items 应为数组");
    assert_eq!(items.len(), 1, "按邮箱关键字应恰好命中 1 条: {body}");
    assert!(
        items[0]["email"]
            .as_str()
            .unwrap_or_default()
            .contains(&needle),
        "命中的应是那个邮箱: {body}"
    );

    delete_user_via_api(&app, &token, uid).await;
}

/// LIKE 通配符必须被转义：搜 `100%` 若命中全表，这个筛选比不筛选更糟
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn user_list_treats_percent_as_literal_text() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (_, all) = query_users(&app, &token, "page_size=200").await;
    let everyone = all["data"]["total"].as_i64().unwrap_or(0);
    assert!(everyone > 0, "测试前提：库里应有账号");

    // 先造一个邮箱里真的含 `%` 的账号，否则"返回 0 条"也能骗过断言。
    // 注意：用户名走 `validate_username`，不允许 `%`，所以只能用邮箱——
    // 而这恰好说明转义必须在**两边**都做，不能只防用户名那列。
    let name = unique("pct");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": name,
                "email": format!("{name}%40x@example.com"),
                "password": "user1234",
                "roles": ["user"],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "造含 % 的邮箱失败: {body}");
    let uid = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("响应里没有可解析的用户 id: {body}"));

    // 只搜一个纯 `%`（URL 编码 %25）
    let (status, body) = query_users(&app, &token, "keyword=%25&page_size=200").await;
    assert_eq!(status, StatusCode::OK, "查询失败: {body}");
    let items = body["data"]["items"].as_array().expect("items 应为数组");

    // 断言**性质**而不是"恰好 1 条"：每一条返回结果都必须真的含字面 `%`。
    // 若 `%` 被当成通配符，这里会混进大量不含 `%` 的账号而立刻被抓到。
    //
    // 为什么不写成 `total == 1`：测试库是长期存在的共享库，任何一次
    // **失败的运行**都会因 panic 跳过清理而留下脏数据，之后 `total == 1`
    // 就永远红了——一个只会因环境脏而失败的断言，比没有断言更坏。
    assert!(
        !items.is_empty(),
        "刚造的那个含 `%` 的账号应能被搜到: {body}"
    );
    for item in items {
        let name = item["username"].as_str().unwrap_or_default();
        let email = item["email"].as_str().unwrap_or_default();
        assert!(
            name.contains('%') || email.contains('%'),
            "搜 `%` 返回了不含字面 `%` 的行: {name} / {email}——通配符未转义"
        );
    }
    assert!(
        (items.len() as i64) < everyone,
        "搜单个 `%` 竟返回 {}/{} 条",
        items.len(),
        everyone
    );

    delete_user_via_api(&app, &token, uid).await;
}

/// 下划线同样是 LIKE 通配符：`zz_` 未转义会连 `zzX` 一起命中
///
/// 造两个账号：`zz_<token>` 与 `zzX<token>`。搜 `zz_<token>`，
/// 转义正确时只命中前者（1 条），未转义时两个都中（2 条）。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn user_list_treats_underscore_as_literal_text() {
    let app = app().await;
    let token = admin_token(&app).await;

    let token_part = unique("u")[3..].to_string();
    let with_us = format!("zz_{token_part}");
    let with_any = format!("zzX{token_part}");
    let uid_us = mkuser(&app, &token, &with_us).await;
    let uid_any = mkuser(&app, &token, &with_any).await;

    let (status, body) = query_users(&app, &token, &format!("keyword={with_us}")).await;
    assert_eq!(status, StatusCode::OK, "查询失败: {body}");
    assert_eq!(
        body["data"]["total"].as_i64(),
        Some(1),
        "搜 {with_us} 应只命中 {with_us}，不该顺带命中 {with_any}——`_` 被当成了通配符"
    );

    delete_user_via_api(&app, &token, uid_us).await;
    delete_user_via_api(&app, &token, uid_any).await;
}

/// 空关键字等同不过滤：搜索框清空后前端会发空串，
/// 若当成关键字就会筛出零条，看起来像"搜不到人"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_empty_keyword_means_no_filtering() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (_, base) = query_users(&app, &token, "page_size=200").await;
    let (_, blank) = query_users(&app, &token, "keyword=&page_size=200").await;
    assert_eq!(
        base["data"]["total"], blank["data"]["total"],
        "空关键字与不过滤的 total 应一致"
    );

    let (_, ws) = query_users(&app, &token, "keyword=%20%20&page_size=200").await;
    assert_eq!(
        base["data"]["total"], ws["data"]["total"],
        "纯空白关键字应等同不过滤"
    );
}

/// 查不到时返回空列表而不是全部——反向判据，防止"过滤条件写反了"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_keyword_that_matches_nobody_returns_an_empty_list() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = query_users(&app, &token, "keyword=zzz_no_such_user_zzz").await;
    assert_eq!(status, StatusCode::OK, "查询失败: {body}");
    assert_eq!(
        body["data"]["total"].as_i64(),
        Some(0),
        "不存在的关键字应返回 0 条: {body}"
    );
    assert!(
        body["data"]["items"].as_array().unwrap().is_empty(),
        "0 条时 items 应为空数组"
    );
}

/// 核心契约：未知查询参数必须 400，而不是被静默丢弃
///
/// 这是本版真正要防的复发路径：只要 `serde` 还默认忽略未知字段，
/// 将来新增任何筛选条件都会再次悄无声息地失效。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_unknown_user_list_filter_is_rejected_loudly() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = query_users(&app, &token, "departmnt=rd").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "拼错的筛选条件应报 400 而不是被静默忽略: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("departmnt"),
        "报错信息应指名那个字段: {body}"
    );
}

// ──────────────────────────────────────────────
// v0.10.0：审计日志筛选 + 导出截断明示
// ──────────────────────────────────────────────

async fn query_logs(app: &Router, token: &str, query: &str) -> (StatusCode, Value) {
    send(
        app,
        request(
            "GET",
            &format!("/api/admin/audit-logs?{query}"),
            Some(token),
            None,
        ),
    )
    .await
}

/// 轮询直到"目标日志出现"为止
///
/// 审计中间件是 `tokio::spawn` 异步落库的（见 `middleware::audit_log`），
/// 所以"刚发完写请求就查"存在竞态：查询完全可能跑在 INSERT 之前。
/// 这里沿用 `admin_requests_are_written_to_audit_log` 的重试写法，
/// 不让这些测试靠运气通过。
///
/// `want` 必须指向**本次测试自己造的那条日志**（例如含特定 role_id 的 action），
/// 不能只判 `items` 非空——查询自身的 GET 也会被记一条日志，
/// 用"非空"当判据会自证成功。
async fn wait_for_logs(
    app: &Router,
    token: &str,
    query: &str,
    want: impl Fn(&Value) -> bool,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..20 {
        let (status, body) = query_logs(app, token, query).await;
        assert_eq!(status, StatusCode::OK, "查询失败: {body}");
        if want(&body) {
            return body;
        }
        last = body;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("等待审计日志超时（约 2s）: query={query}, last={last}");
}

/// 用真实写操作造出可辨认的日志，再按 username 筛出来
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn audit_logs_filter_by_username() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 关键在于造一条**别的用户**的日志。全库日志都出自 admin 时，
    // "筛选=全量"和"筛选生效"观察上完全一样，测试就成了自证。
    let other = unique("auditother");
    let other_id = mkuser(&app, &token, &other).await;
    let other_token = activated_token(&app, &other, "user1234").await;

    // 普通用户打管理接口 → 403。审计中间件在认证之内，403 同样留痕。
    let (status, _) = send(
        &app,
        request("GET", "/api/admin/users", Some(&other_token), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "普通用户本该被权限守卫拦下；变了说明造日志的前提不成立"
    );

    let body = wait_for_logs(
        &app,
        &token,
        &format!("username={other}&page_size=50"),
        |body| {
            !body["data"]["items"]
                .as_array()
                .is_none_or(|items| items.is_empty())
        },
    )
    .await;
    let items = body["data"]["items"].as_array().expect("items 应为数组");
    assert!(!items.is_empty(), "按该用户名筛应能查到它自己的日志");
    for item in items {
        assert_eq!(
            item["username"].as_str(),
            Some(other.as_str()),
            "按 username={other} 筛选却返回了别人的日志: {item}"
        );
    }

    // 反向判据：同一条日志在 username=admin 下必须不出现
    let (_, admin_view) = query_logs(&app, &token, "username=admin&page_size=200").await;
    let leaked = admin_view["data"]["items"]
        .as_array()
        .expect("items 应为数组")
        .iter()
        .any(|it| it["username"].as_str() == Some(other.as_str()));
    assert!(
        !leaked,
        "username=admin 的结果里混进了 {other} 的日志，说明 username 筛选没生效"
    );

    delete_user_via_api(&app, &token, other_id).await;
}

/// 按状态码精确筛选：只查 404 应不含 200 的记录
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn audit_logs_filter_by_status_code() {
    let app = app().await;
    let token = admin_token(&app).await;

    // 404：删除一个不存在的用户。
    // 注意不能用 `GET /api/admin/users/{id}`——那条路由只注册了 PUT/DELETE，
    // GET 会得到 405 Method Not Allowed，测的就不是状态码筛选了。
    let (status, _) = send(
        &app,
        request(
            "DELETE",
            "/api/admin/users/00000000-0000-4000-8000-000000000009",
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "造 404 日志失败，后续断言会失去意义"
    );

    let body = wait_for_logs(&app, &token, "status_code=404&page_size=50", |body| {
        !body["data"]["items"]
            .as_array()
            .is_none_or(|items| items.is_empty())
    })
    .await;
    let items = body["data"]["items"].as_array().expect("items 应为数组");
    for item in items {
        assert_eq!(
            item["status_code"].as_i64(),
            Some(404),
            "按 status_code=404 筛选却返回了其它状态码的日志: {item}"
        );
    }
}

/// 按 action 模糊筛选：只含某段路径的日志才该命中
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn audit_logs_filter_by_action() {
    let app = app().await;
    let token = admin_token(&app).await;

    let role_name = unique("actf");
    let role_id = create_role_via_api(&app, &token, &role_name).await;
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/roles/{role_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "造日志失败: {body}");

    // role_id 是本次测试独有的，用它当筛选值：
    // 一旦筛选被丢弃，页面上必然混进不相关的行，逐行断言即可拆穿。
    let needle = role_id.to_string();
    let body = wait_for_logs(
        &app,
        &token,
        &format!("action={needle}&page_size=50"),
        |body| {
            body["data"]["items"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|it| it["action"].as_str().is_some_and(|a| a.contains(&needle)))
            })
        },
    )
    .await;
    let items = body["data"]["items"].as_array().expect("items 应为数组");
    for item in items {
        let action = item["action"].as_str().unwrap_or_default();
        assert!(
            action.contains(&needle),
            "按 action={needle} 筛选却返回了不相关的行: {action}"
        );
    }
}

/// 筛不到时返回空列表而不是全部——反向判据
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_audit_filter_that_matches_nothing_returns_an_empty_list() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = query_logs(&app, &token, "username=zzz_nobody_zzz").await;
    assert_eq!(status, StatusCode::OK, "查询失败: {body}");
    assert_eq!(body["data"]["total"].as_i64(), Some(0), "{body}");
    assert!(
        body["data"]["items"].as_array().unwrap().is_empty(),
        "筛不到时应返回空数组: {body}"
    );
}

/// 核心契约：审计日志的未知参数同样必须 400
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_unknown_audit_log_filter_is_rejected_loudly() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = query_logs(&app, &token, "user_name=admin").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "拼错的筛选条件应报 400: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("user_name"),
        "报错应指名那个字段: {body}"
    );
    // 统一响应格式：必须是 JSON 且带 code/message/data
    assert_eq!(
        body["code"].as_i64(),
        Some(400),
        "应遵守统一响应格式: {body}"
    );
    assert!(body.get("message").is_some(), "应含 message: {body}");
}

/// 导出的筛选条件必须真的生效，且截断状态随响应头返回
///
/// 原实现硬编码 `LIMIT 10000` 且不告知任何人。这里断言两件事：
/// 1. 导出带筛选时，条数与筛选结果一致（不是全量）
/// 2. 响应头里有 `x-export-row-count` / `x-export-truncated` / `x-export-max-rows`
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn audit_export_honors_filters_and_reports_truncation() {
    let app = app().await;
    let token = admin_token(&app).await;

    let role_name = unique("expf");
    let role_id = create_role_via_api(&app, &token, &role_name).await;
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/roles/{role_id}"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "造日志失败: {body}");

    // 带筛选导出
    let response = app
        .clone()
        .oneshot(request(
            "GET",
            "/api/admin/logs/audit/export?username=zzz_nobody_zzz",
            Some(&token),
            None,
        ))
        .await
        .expect("导出请求失败");
    assert_eq!(response.status(), StatusCode::OK, "导出应成功");

    // 截断状态必须机器可读
    let row_count = response
        .headers()
        .get("x-export-row-count")
        .and_then(|v| v.to_str().ok())
        .expect("应回传 x-export-row-count");
    assert_eq!(
        row_count, "0",
        "筛选到 0 条时导出行数应为 0（说明筛选真的生效，而非仍导出全量）"
    );
    assert_eq!(
        response
            .headers()
            .get("x-export-truncated")
            .and_then(|v| v.to_str().ok()),
        Some("false"),
        "未触顶时 truncated 应为 false"
    );
    assert!(
        response.headers().get("x-export-max-rows").is_some(),
        "应回传上限，前端据此告知用户"
    );
}

// ──────────────────────────────────────────────
// v0.10.0：角色列表分页（破坏性 API 变更）
//
// 断言写成"性质"而不是"条数"：库里有多少角色取决于前面用例留下了什么，
// 写死 `total == 5` 会被脏数据永久打红。
// ──────────────────────────────────────────────

/// 角色列表返回分页对象，逐页取回与一次取全量**完全一致**
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn role_list_pages_over_the_same_set_as_one_big_page() {
    let app = app().await;
    let token = admin_token(&app).await;

    let mut created = vec![];
    for _ in 0..3 {
        created.push(create_role_via_api(&app, &token, &unique("pgrole")).await);
    }

    let (status, whole) = send(
        &app,
        request("GET", "/api/admin/roles?page_size=200", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{whole}");

    // 形状：分页对象，不是裸数组（v0.10.0 的破坏性变更）
    assert!(
        !whole["data"].is_array(),
        "角色列表不应再返回裸数组: {whole}"
    );
    let all_ids: Vec<String> = whole["data"]["items"]
        .as_array()
        .expect("data.items 应为数组")
        .iter()
        .map(|r| r["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        whole["data"]["total"].as_i64(),
        Some(all_ids.len() as i64),
        "total 必须与 items 长度一致，否则分页会错乱: {whole}"
    );
    assert!(whole["data"]["total_pages"].as_i64().unwrap_or(0) >= 1);

    // 逐页取回，集合必须与一次取全量一致：既不重也不漏
    let mut paged: Vec<String> = vec![];
    let mut page = 1;
    loop {
        let (status, body) = send(
            &app,
            request(
                "GET",
                &format!("/api/admin/roles?page={page}&page_size=2"),
                Some(&token),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let items = body["data"]["items"]
            .as_array()
            .expect("data.items 应为数组");
        if items.is_empty() {
            break;
        }
        for r in items {
            paged.push(r["id"].as_str().unwrap_or_default().to_string());
        }
        page += 1;
        assert!(page < 200, "翻页没有收敛，可能陷入死循环");
    }

    let mut sorted_paged = paged.clone();
    sorted_paged.sort();
    sorted_paged.dedup();
    assert_eq!(
        sorted_paged.len(),
        paged.len(),
        "翻页取回了重复的角色，说明分页缺稳定排序: {paged:?}"
    );

    let mut sorted_all = all_ids.clone();
    sorted_all.sort();
    assert_eq!(
        sorted_paged, sorted_all,
        "逐页取回的角色集合与一次取全量不一致"
    );

    for id in created {
        assert!(
            all_ids.contains(&id.to_string()),
            "刚建的角色 {id} 不该从列表里消失"
        );
        let _ = delete_role(&app, &token, id).await;
    }
}

/// 角色列表的未知参数同样必须 400，不能静默忽略
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_unknown_role_list_filter_is_rejected_loudly() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request("GET", "/api/admin/roles?pageSize=10", Some(&token), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "驼峰拼错的分页参数应报 400: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("pageSize"),
        "报错应指名那个字段: {body}"
    );
}

// ──────────────────────────────────────────────
// v0.10.0：契约测试——前端 query 参数 vs 后端 DTO 字段
//
// 这一族问题的特点是**从不报错**：筛选栏摆着、参数也确实发出去了，
// 后端不认识就静默丢弃，界面表现为"搜索没反应"。
// 端到端测试抓不到它（请求确实成功、只是筛不出东西），
// 只能靠"把两边的字段名拿来对照"这种静态契约。
// ──────────────────────────────────────────────

/// 取出 `pub struct <name> { … }` 的字段名
fn rust_struct_fields(source: &str, name: &str) -> Vec<String> {
    let marker = format!("pub struct {name} {{");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("在源文件里找不到 `pub struct {name}`"))
        + marker.len();
    let body = &source[start..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("`pub struct {name}` 的结构体没有正确闭合"));

    body[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub "))
        .filter_map(|rest| rest.split(':').next())
        .map(|field| field.trim().to_string())
        .filter(|field| !field.is_empty())
        .collect()
}

/// 取出一段 TS 类型文本里的字段名
///
/// 同时支持 `export interface X { a?: number }` 与内联的
/// `list(params: { a?: number; b?: string })`——两种写法前端都真实在用。
fn ts_field_names(region: &str) -> Vec<String> {
    region
        .split([';', '\n', ','])
        .filter_map(|fragment| {
            let fragment = fragment.trim();
            let head: String = fragment
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let rest = fragment[head.len()..].trim_start();
            // 只接受 `name:` 与 `name?:` 两种字段形态，顺带滤掉注释残片
            if let Some(after_question) = rest.strip_prefix('?') {
                after_question.trim_start().starts_with(':').then_some(head)
            } else {
                rest.starts_with(':').then_some(head)
            }
        })
        .filter(|field| !field.is_empty())
        .collect()
}

/// 取出 `export interface <name> { … }` 的字段名
fn ts_interface_fields(source: &str, name: &str) -> Vec<String> {
    let marker = format!("export interface {name} {{");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("在前端源文件里找不到 `export interface {name}`"))
        + marker.len();
    let rest = &source[start..];
    let end = rest
        .find('}')
        .unwrap_or_else(|| panic!("`export interface {name}` 没有正确闭合"));
    ts_field_names(&rest[..end])
}

/// 取出内联对象类型的字段名（如 `list(params: { page?: number })`）
fn ts_inline_fields(source: &str, marker: &str) -> Vec<String> {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("在前端源文件里找不到 `{marker}`"))
        + marker.len();
    let rest = &source[start..];
    let end = rest
        .find('}')
        .unwrap_or_else(|| panic!("`{marker}` 后的内联类型没有正确闭合"));
    ts_field_names(&rest[..end])
}

/// 前端发出的 query 字段必须与后端 DTO 的字段**逐字对齐**
#[test]
fn frontend_query_params_match_backend_dto_fields() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // (前端文件, 前端参数写法, 后端文件, 后端 DTO)
    let pairs: &[(&str, &str, &str, &str)] = &[
        (
            "frontend/src/api/user.ts",
            "list(params: {",
            "src/controller/user.rs",
            "UserListParams",
        ),
        (
            "frontend/src/api/role.ts",
            "list(params: {",
            "src/controller/role.rs",
            "RoleListParams",
        ),
        (
            "frontend/src/api/audit.ts",
            "export interface AuditLogListParams {",
            "src/controller/demo.rs",
            "AuditLogQuery",
        ),
        (
            "frontend/src/api/menu.ts",
            "{ params: {",
            "src/controller/menu.rs",
            "MenuQuery",
        ),
        (
            "frontend/src/api/dict.ts",
            "{ params: {",
            "src/controller/dict.rs",
            "DictItemQuery",
        ),
    ];

    let mut problems: Vec<String> = vec![];

    for (fe_file, fe_marker, be_file, be_struct) in pairs {
        let fe_source = std::fs::read_to_string(root.join(fe_file))
            .unwrap_or_else(|e| panic!("读取 {fe_file} 失败: {e}"));
        let be_source = std::fs::read_to_string(root.join(be_file))
            .unwrap_or_else(|e| panic!("读取 {be_file} 失败: {e}"));

        let fe_fields = if let Some(name) = fe_marker
            .strip_prefix("export interface ")
            .and_then(|rest| rest.strip_suffix(" {"))
        {
            ts_interface_fields(&fe_source, name)
        } else {
            ts_inline_fields(&fe_source, fe_marker)
        };
        let be_fields = rust_struct_fields(&be_source, be_struct);

        // 危险方向：前端发了后端不认识的字段——会被静默丢弃或 400
        let unknown_to_backend: Vec<&String> = fe_fields
            .iter()
            .filter(|f| !be_fields.contains(f))
            .collect();
        if !unknown_to_backend.is_empty() {
            problems.push(format!(
                "{fe_file} 发出后端 {be_struct} 不认识的字段: {unknown_to_backend:?}（会静默失效或 400）"
            ));
        }

        // 另一个方向：后端有、前端从不发的字段——筛选入口可能还没接上
        let never_sent: Vec<&String> = be_fields
            .iter()
            .filter(|f| !fe_fields.contains(f))
            .collect();
        if !never_sent.is_empty() {
            problems.push(format!(
                "{be_file} 的 {be_struct} 有前端从未发送的字段: {never_sent:?}（筛选入口可能没接上）"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "前后端 query 字段对不齐:\n  {}",
        problems.join("\n  ")
    );
}

/// 后端每个 query DTO 都必须 `deny_unknown_fields`
///
/// 这条不靠人列清单：新增 `*Query` / `*Params` 结构体时自动纳入检查。
/// 忘了加就会退回"静默丢弃"，而那正是本版要消灭的行为。
#[test]
fn every_query_dto_rejects_unknown_fields() {
    let controller_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controller");

    let mut offenders: Vec<String> = vec![];
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&controller_dir).expect("读取 src/controller 失败") {
        let path = entry.expect("目录项读取失败").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读取 controller 源文件失败");
        let file = path.file_name().unwrap().to_string_lossy().to_string();

        let mut cursor = 0usize;
        while let Some(start) = source[cursor..].find("pub struct ") {
            let abs_start = cursor + start;
            let after = &source[abs_start + "pub struct ".len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            cursor = abs_start + "pub struct ".len() + name.len();

            if !(name.ends_with("Query") || name.ends_with("Params")) {
                continue;
            }
            checked += 1;

            // 属性写在结构体上一行，取紧邻的非空行即可
            let attr = source[..abs_start]
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim();
            if attr != "#[serde(deny_unknown_fields)]" {
                offenders.push(format!(
                    "{file}::{name} 缺少 #[serde(deny_unknown_fields)]（紧邻的上一行是 `{attr}`）"
                ));
            }
        }
    }

    assert!(
        checked >= 5,
        "应至少校验 5 个 query DTO，实际 {checked}——是扫描逻辑失效了"
    );
    assert!(
        offenders.is_empty(),
        "以下 query DTO 会静默丢弃未知字段:\n  {}",
        offenders.join("\n  ")
    );
}

// ──────────────────────────────────────────────
// v0.11.0：登录可审计 + 自助改密 + 口令策略
// ──────────────────────────────────────────────

/// 该用户在某 action 下的全部审计行
fn rows_for(body: &Value, username: &str, action: &str) -> Vec<Value> {
    body["data"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            r["username"].as_str() == Some(username) && r["action"].as_str() == Some(action)
        })
        .collect()
}

/// 登录成功落审计，且**带 client_ip**
///
/// 回归的是 v0.11.0 的起点：`/api/auth/login` 在 `public_routes` 里，
/// 没有挂 `audit_log_middleware`，此前成功与失败都不进 `audit_logs`。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_successful_login_is_audited_with_its_client_ip() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("loginsuccess");
    mkuser(&app, &admin, &name).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "登录应成功: {body}");

    let logs = wait_for_logs(
        &app,
        &admin,
        &format!("action=AUTH_LOGIN_SUCCESS&username={name}"),
        |b| !rows_for(b, &name, "AUTH_LOGIN_SUCCESS").is_empty(),
    )
    .await;

    let rows = rows_for(&logs, &name, "AUTH_LOGIN_SUCCESS");
    assert_eq!(rows.len(), 1, "一次登录应恰好一条成功审计: {rows:?}");
    let row = &rows[0];
    assert_eq!(row["method"].as_str(), Some("POST"));
    assert_eq!(row["path"].as_str(), Some("/api/auth/login"));
    assert_eq!(row["status_code"].as_u64(), Some(200));
    // client_ip 是"谁从哪登录"的唯一来源，缺了这条追溯就断了一半
    assert!(
        !row["client_ip"].as_str().unwrap_or("").is_empty(),
        "成功登录必须记下 client_ip: {row}"
    );
    assert_eq!(
        row["user_id"].as_str(),
        Some(user_id_by_username(&name).await.to_string().as_str()),
        "成功登录应能关联到用户 id"
    );
}

/// 口令错误的登录**也要落审计**，且记的是"尝试登录时提交的名字"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_wrong_password_login_is_audited() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("loginfail");
    mkuser(&app, &admin, &name).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "definitely-not-it" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "口令错误应 401: {body}");

    let logs = wait_for_logs(
        &app,
        &admin,
        &format!("action=AUTH_LOGIN_FAILURE&username={name}"),
        |b| !rows_for(b, &name, "AUTH_LOGIN_FAILURE").is_empty(),
    )
    .await;

    let rows = rows_for(&logs, &name, "AUTH_LOGIN_FAILURE");
    let row = &rows[0];
    assert_eq!(row["status_code"].as_u64(), Some(401));
    assert!(
        row["result"].as_str().unwrap_or("").contains("口令"),
        "应写明失败原因，实际 {:?}",
        row["result"]
    );
    assert!(
        !row["client_ip"].as_str().unwrap_or("").is_empty(),
        "失败登录同样要记 client_ip（爆破溯源）: {row}"
    );
}

/// 对**不存在的账号**登录也要留痕——那正是爆破的证据
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_login_for_an_unknown_account_is_audited_too() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let ghost = unique("ghostacct");

    let (status, _) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": ghost, "password": "whatever123" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let logs = wait_for_logs(
        &app,
        &admin,
        &format!("action=AUTH_LOGIN_FAILURE&username={ghost}"),
        |b| !rows_for(b, &ghost, "AUTH_LOGIN_FAILURE").is_empty(),
    )
    .await;

    let row = &rows_for(&logs, &ghost, "AUTH_LOGIN_FAILURE")[0];
    assert!(
        row["result"].as_str().unwrap_or("").contains("账号"),
        "应写明账号不存在，实际 {:?}",
        row["result"]
    );
    // 查无此人时 user_id 必须为空——不能把一个不存在的 id 写进去
    assert!(row["user_id"].is_null(), "查无此人时不应有 user_id: {row}");
}

/// 审计筛选必须**双向收窄**，否则等于没筛
///
/// 只断言"目标行在里面"是自证：不过滤时全量也满足这个条件。
/// 这里同时断言"别的用户的行不在里面"。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn login_audit_filters_narrow_in_both_directions() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let mine = unique("narrow_mine");
    let other = unique("narrow_other");
    mkuser(&app, &admin, &mine).await;
    mkuser(&app, &admin, &other).await;

    for name in [&mine, &other] {
        send(
            &app,
            request(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({ "username": name, "password": "user1234" })),
            ),
        )
        .await;
    }

    let logs = wait_for_logs(
        &app,
        &admin,
        &format!("action=AUTH_LOGIN_SUCCESS&username={mine}"),
        |b| !rows_for(b, &mine, "AUTH_LOGIN_SUCCESS").is_empty(),
    )
    .await;

    let mine_rows = rows_for(&logs, &mine, "AUTH_LOGIN_SUCCESS");
    assert!(!mine_rows.is_empty(), "自己的成功日志应在结果里");
    assert!(
        rows_for(&logs, &other, "AUTH_LOGIN_SUCCESS").is_empty(),
        "按 {mine} 筛选时不该带出 {other} 的日志: {logs}"
    );
}

/// 注册落审计
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn registration_is_audited() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("registered");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": name,
                "email": format!("{name}@example.com"),
                "password": "register1A",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "注册应成功: {body}");

    let logs = wait_for_logs(
        &app,
        &admin,
        &format!("action=AUTH_REGISTER&username={name}"),
        |b| !rows_for(b, &name, "AUTH_REGISTER").is_empty(),
    )
    .await;

    let row = &rows_for(&logs, &name, "AUTH_REGISTER")[0];
    assert_eq!(row["path"].as_str(), Some("/api/auth/register"));
    assert!(
        !row["client_ip"].as_str().unwrap_or("").is_empty(),
        "注册也要记 client_ip: {row}"
    );
}

/// 审计 `username` 列只有 50 字符，超长用户名不能让登录变成 500
///
/// 这是"审计反过来变成拒绝服务入口"的回归：不截断的话
/// 一个 200 字符的用户名会让 INSERT 报 `value too long`，
/// 于是"口令错误"被升级成 500。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_overlong_username_on_login_does_not_break_the_audit_write() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let huge = "u".repeat(300);
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": huge, "password": "whatever123" })),
        ),
    )
    .await;
    // 关键：必须是业务错误 401，而不是审计写入失败导致的 500
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "超长用户名应得 401，实际 {status}: {body}"
    );

    // 审计确实落库了（被截断到 50）
    let logs = wait_for_logs(&app, &admin, "action=AUTH_LOGIN_FAILURE", |b| {
        b["data"]["items"]
            .as_array()
            .map(|items| {
                items.iter().any(|r| {
                    r["username"].as_str().map(|u| u.len()) == Some(50)
                        && r["username"].as_str().map(|u| u.starts_with('u')) == Some(true)
                })
            })
            .unwrap_or(false)
    })
    .await;
    assert!(
        logs["data"]["items"]
            .as_array()
            .is_some_and(|i| !i.is_empty()),
        "超长用户名也应留下一条被截断的审计"
    );
}

/// 管理员建号 → 登录拿到**受限令牌** → 业务接口被拦 → 改密 → 恢复正常
///
/// 这是 v0.11.0 的核心链路，一条测试走完。
/// 特别之处：拦截发生在**后端中间件**，
/// 不是靠前端跳转——只让前端跳改密页的话，令牌本身仍能调任何接口，
/// 那等于把权限校验交给界面，与 v0.10.0 关掉的"界面替后端承诺"同类。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn an_admin_created_user_is_confined_to_changing_password() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("mustchange");
    mkuser(&app, &admin, &name).await;
    // 夹具刚把标记清了，这里显式置回来，走真实的"管理员建号"语义
    sqlx::query("UPDATE users SET must_change_password = TRUE WHERE username = $1")
        .bind(&name)
        .execute(&pool().await)
        .await
        .expect("置强制改密标记失败");

    // ── 登录：拿到受限令牌，且响应明说自己是受限的
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "登录应成功: {body}");
    assert_eq!(
        body["data"]["must_change_password"].as_bool(),
        Some(true),
        "登录响应应明说这是受限令牌: {body}"
    );
    let restricted = body["data"]["token"]
        .as_str()
        .expect("缺少 token")
        .to_string();

    // ── 放行的：看自己、改密
    //
    // 注意这里**故意用错的旧口令**：请求要走到 handler 才会得到 400，
    // 若被中间件拦下则是 403——两者可区分，才说明"放行"是真的。
    // 不能在这里真的改密：那会吊销掉本令牌，后面所有断言都变成 401。
    let (status, _) = send(
        &app,
        request("GET", "/api/auth/me", Some(&restricted), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "/api/auth/me 应放行，实际 {status}");

    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&restricted),
            Some(json!({ "old_password": "wrong-on-purpose", "new_password": "ChangedPass1" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "改密接口应放行到 handler（因旧口令错而 400），实际 {status}: {body}"
    );

    // ── 拦住的：任何业务接口
    for path in [
        "/api/auth/permissions",
        "/api/admin/users",
        "/api/admin/monitor/system",
    ] {
        let (status, body) = send(&app, request("GET", path, Some(&restricted), None)).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "受限令牌不该能访问 {path}: {body}"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap_or("")
                .contains("修改初始密码"),
            "403 提示要说清该做什么，实际 {:?}",
            body["message"]
        );
    }

    // ── 登出放行，且排在最后：它会注销令牌，放在前面后续断言就全失效了
    let (status, _) = send(
        &app,
        request("POST", "/api/auth/logout", Some(&restricted), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "/api/auth/logout 应放行，实际 {status}"
    );
}

/// 改密之后：旧令牌全部失效，重新登录拿到正常令牌
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn changing_password_revokes_every_existing_session() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("pwchange");
    mkuser(&app, &admin, &name).await;

    // 两个设备各登录一次
    let phone = activated_token(&app, &name, "user1234").await;
    let laptop = activated_token(&app, &name, "user1234").await;
    assert_ne!(phone, laptop, "两次登录应拿到不同令牌");

    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&phone),
            Some(json!({ "old_password": "user1234", "new_password": "FreshPass99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改密应成功: {body}");

    // 改密的那一端也要重新登录——口令变了凭据就作废
    for (label, tok) in [("发起改密的设备", &phone), ("另一个设备", &laptop)] {
        let (status, _) = send(&app, request("GET", "/api/auth/me", Some(tok), None)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{label} 的旧令牌应在改密后失效，实际 {status}"
        );
    }

    // 新口令可登录，旧口令不可
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "FreshPass99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新口令应能登录: {body}");
    let (status, _) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "旧口令应失效");
}

/// 必须验旧口令：只验复杂度的话，令牌被劫持即可永久占据账号
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn changing_password_requires_the_current_password() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("pwold");
    mkuser(&app, &admin, &name).await;
    let tok = activated_token(&app, &name, "user1234").await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&tok),
            Some(json!({ "old_password": "not-my-password", "new_password": "FreshPass99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "旧口令错误应 400: {body}");

    // 口令没被改掉
    let (status, _) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "原口令应仍然有效");
}

/// 新旧口令相同必须拒绝：否则改密是空操作却回了"成功"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn changing_password_rejects_reusing_the_current_one() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("pwsame");
    mkuser(&app, &admin, &name).await;
    let tok = activated_token(&app, &name, "user1234").await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&tok),
            Some(json!({ "old_password": "user1234", "new_password": "user1234" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "相同口令应被拒: {body}");
    assert!(
        body["message"].as_str().unwrap_or("").contains("相同"),
        "提示要说清原因，实际 {:?}",
        body["message"]
    );
}

/// 自助改密**不能**顺带改自己的角色或启用状态
///
/// 判据落在"多传字段被指名拒绝"，而不是"传了也没生效"——
/// 后者满足于调用方误以为成功，与 v0.10.0 关掉的假接口同类。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn changing_password_cannot_also_grant_itself_roles() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("pwnope");
    mkuser(&app, &admin, &name).await;
    let tok = activated_token(&app, &name, "user1234").await;
    let uid = user_id_by_username(&name).await;

    for extra in [
        json!({ "old_password": "user1234", "new_password": "FreshPass99", "roles": ["admin"] }),
        json!({ "old_password": "user1234", "new_password": "FreshPass99", "is_active": true }),
        json!({ "old_password": "user1234", "new_password": "FreshPass99", "user_id": uid }),
    ] {
        let (status, body) = send(
            &app,
            request("PUT", "/api/auth/password", Some(&tok), Some(extra.clone())),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "多传 {extra} 应被 400 拒绝: {body}"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap_or("")
                .contains("unknown field"),
            "应指名是哪个字段不认，实际 {:?}",
            body["message"]
        );
    }

    // 角色与状态都没被改动
    assert_eq!(
        role_names_in_db(uid).await,
        vec!["user".to_string()],
        "自助改密不得改动角色"
    );
    let active: bool = sqlx::query_scalar("SELECT is_active FROM users WHERE id = $1")
        .bind(uid)
        .fetch_one(&pool().await)
        .await
        .expect("查询 is_active 失败");
    assert!(active, "自助改密不得改动启用状态");
}

/// 公开注册的用户**不受**强制改密约束
///
/// 回归 v0.11.0 的核心约束"不叠加第二次强制登出"：
/// 用户自己设的口令不该被要求再改一次。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn self_registered_users_are_not_forced_to_change_password() {
    let app = app().await;

    let name = unique("selfreg");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": name,
                "email": format!("{name}@example.com"),
                "password": "selfreg123",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "注册应成功: {body}");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "selfreg123" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "登录应成功: {body}");
    assert_eq!(
        body["data"]["must_change_password"].as_bool(),
        Some(false),
        "自己设的口令不该被要求改掉: {body}"
    );
    let tok = body["data"]["token"].as_str().expect("缺少 token");

    // 立刻就能正常用业务接口
    let (status, _) = send(
        &app,
        request("GET", "/api/auth/permissions", Some(tok), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自注册用户不该被拦在改密页");
}

/// 弱口令在"设置口令"时被拒
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn weak_passwords_are_rejected_when_being_set() {
    let app = app().await;

    // 12345678：长度够，但只有一个字符类
    let name = unique("weakpw");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": name,
                "email": format!("{name}@example.com"),
                "password": "12345678",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "弱口令应被拒: {body}");
    assert!(
        body["message"].as_str().unwrap_or("").contains("复杂度"),
        "提示应说明是复杂度问题，实际 {:?}",
        body["message"]
    );
    assert!(!user_exists(&name).await, "被拒的注册不应留下半个账号");
}

/// 管理员重置口令后，该用户下次登录必须改密
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn resetting_a_password_arms_the_forced_change_flag() {
    let app = app().await;
    let admin = admin_token(&app).await;

    let name = unique("resetpw");
    let uid = mkuser(&app, &admin, &name).await;

    // 先自助改一次，把标记清掉，验证"重置会重新置位"
    let tok = activated_token(&app, &name, "user1234").await;
    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&tok),
            Some(json!({ "old_password": "user1234", "new_password": "SelfChosen1" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自助改密应成功: {body}");

    // 管理员重置
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/users/{uid}/reset-password"),
            Some(&admin),
            Some(json!({ "password": "AdminSet99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重置应成功: {body}");

    let flag: bool = sqlx::query_scalar("SELECT must_change_password FROM users WHERE id = $1")
        .bind(uid)
        .fetch_one(&pool().await)
        .await
        .expect("查询标记失败");
    assert!(flag, "管理员重置后必须重新要求改密");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "username": name, "password": "AdminSet99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重置后的口令应能登录: {body}");
    assert_eq!(
        body["data"]["must_change_password"].as_bool(),
        Some(true),
        "重置后的登录应拿到受限令牌: {body}"
    );
}

/// 前后端口令策略**必须给出同一结论**
///
/// 规则在前端 `utils/password.ts` 与后端 `validation.rs` 各存一份：
/// 前端那份只为即时反馈，后端才是裁决方。两份必然有漂移风险，
/// 而漂移的后果很具体——界面说"符合要求"，点提交却被后端拒绝。
///
/// 做法是从前端源码里解析出 `PASSWORD_POLICY_CASES` 样例，
/// 用 Rust 的 `validate_password` 跑同一批口令并比对结论。
/// 样例清单本身也是代码里的单一条目，改一处即两侧同步。
#[test]
fn password_policy_agrees_with_the_frontend_copy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("frontend/src/utils/password.ts"))
        .expect("读取 frontend/src/utils/password.ts 失败");

    // **只在数组体内解析**：文件顶部的文档注释里也写着 `{ pw: '...', ok: true }`
    // 这样的示例，全文搜索会把它当成一条真实样例（`...` 当然过不了复杂度）。
    let body_start = source
        .find("export const PASSWORD_POLICY_CASES")
        .expect("前端未导出 PASSWORD_POLICY_CASES");
    let body = &source[body_start..];
    let body_end = body
        .find("\n]")
        .expect("PASSWORD_POLICY_CASES 数组未正常闭合");
    let body = &body[..body_end];

    // 解析 `{ pw: '...', ok: true|false }`
    let mut cases: Vec<(String, bool)> = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("{ pw: '") {
        let after = &rest[at + "{ pw: '".len()..];
        let Some(end_quote) = after.find('\'') else {
            break;
        };
        let pw = after[..end_quote].to_string();
        let tail = &after[end_quote + 1..];
        let Some(ok_at) = tail.find(", ok: ") else {
            break;
        };
        let verdict = &tail[ok_at + ", ok: ".len()..];
        let verdict_len = if verdict.starts_with("true") {
            4
        } else if verdict.starts_with("false") {
            5
        } else {
            panic!("无法解析 ok 字段: {verdict}");
        };
        cases.push((pw, verdict.starts_with("true")));
        rest = &tail[ok_at + ", ok: ".len() + verdict_len..];
    }

    assert!(
        cases.len() >= 10,
        "从前端解析到的口令样例过少（{} 条），样例表可能被改坏",
        cases.len()
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (pw, expected_ok) in &cases {
        let actual_ok = axum_api::utils::validation::validate_password(pw).is_ok();
        if actual_ok != *expected_ok {
            mismatches.push(format!(
                "{pw:?}：前端期望 {expected_ok}，后端实际 {actual_ok}"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "前后端口令策略判定不一致：\n{}",
        mismatches.join("\n")
    );
}

/// 前后端的长度上下限必须一致
///
/// 上面那条比的是"给定口令的结论"，比不出**边界值**本身被改动的情况：
/// 若两侧同时把下限从 8 抬到 10，样例结论可能仍全对。
/// 因此单独锁住两个常量。
#[test]
fn password_length_bounds_match_the_frontend_copy() {
    use axum_api::utils::validation::{PASSWORD_MAX_LEN, PASSWORD_MIN_LEN};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("frontend/src/utils/password.ts"))
        .expect("读取前端口令策略失败");

    let parse = |name: &str| -> usize {
        let marker = format!("export const {name} = ");
        let at = source
            .find(&marker)
            .unwrap_or_else(|| panic!("前端未导出 {name}"));
        let rest = &source[at + marker.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest[..end]
            .parse()
            .unwrap_or_else(|e| panic!("{name} 不是数字: {e}"))
    };

    assert_eq!(
        parse("PASSWORD_MIN_LEN"),
        PASSWORD_MIN_LEN,
        "前端 PASSWORD_MIN_LEN 与后端不一致"
    );
    assert_eq!(
        parse("PASSWORD_MAX_LEN"),
        PASSWORD_MAX_LEN,
        "前端 PASSWORD_MAX_LEN 与后端不一致"
    );
}

// ──────────────────────────────────────────────
// 错误响应格式统一（v0.12.0）
// ──────────────────────────────────────────────

/// 一个端点上要打的坏输入探针
struct BadInputProbe {
    /// 探针指向的端点，形如 `POST /api/admin/users`（用于失败信息）
    endpoint: String,
    /// 造出来的请求
    req: Request<Body>,
    /// 这条探针想证明什么
    intent: &'static str,
}

/// 合法 UUID：路径探针要它**通过**解析，让请求能走到下一层
const VALID_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// 列出 OpenAPI 文档里的每个操作 `(method, path, operation)`
fn openapi_operations() -> Vec<(String, String, Value)> {
    let doc = axum_api::docs::openapi_json();
    let mut ops = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("OpenAPI 的 paths 应为对象") {
        for (method, op) in item.as_object().expect("每个 path 应为操作表") {
            ops.push((method.to_uppercase(), path.clone(), op.clone()));
        }
    }
    // 只按 `(method, path)` 排：第三个元素是 `Value`，没有 `Ord`，
    // 用 `sort()` 会在编译期要求整个三元组可比较
    ops.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ops
}

/// 该操作路径模板里 `format: uuid` 的参数名
///
/// 只认 uuid 是有意的：`{code}` 这类 `String` 参数传什么都解析得出来，
/// 用非 UUID 去探它只会得到 200/404，测不到提取器。判据来自文档里的
/// `schema.format`，不靠手写清单。
fn uuid_path_params(op: &Value) -> Vec<String> {
    op["parameters"]
        .as_array()
        .map(|params| {
            params
                .iter()
                .filter(|p| p["in"] == "path" && p["schema"]["format"] == "uuid")
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 把路径模板里的指定参数替换成给定值
fn substitute(template: &str, names: &[String], value: &str) -> String {
    names.iter().fold(template.to_string(), |acc, name| {
        acc.replace(&format!("{{{name}}}"), value)
    })
}

/// 构造带原始文本响应体的请求
///
/// 现有的 `request()` 收 `Value`，只能发合法 JSON；
/// 而这里要故意发**解析不了的**字节（`{bad json`）和**错的** Content-Type，
/// 所以另建一个入口。
fn raw_request(
    method: &str,
    path: &str,
    token: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("构造请求失败")
}

/// 发送并把响应的 **Content-Type 原文** 与解析后的 body 一起带回
///
/// Content-Type 是本用例的核心判据之一：`{bad json` 在迁移前会得到
/// `text/plain`，而 JSON 信封是 `application/json`。只回解析后的
/// `Value` 会把这个差别抹掉（纯文本解析失败会退化成 `Value::Null`）。
async fn send_raw(app: &Router, req: Request<Body>) -> (StatusCode, String, Value) {
    let response = app.clone().oneshot(req).await.expect("请求执行失败");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("读取响应体失败");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, content_type, value)
}

/// 判定"这个响应是统一错误信封"，把不满足的地方说清楚
fn envelope_violation(
    endpoint: &str,
    intent: &str,
    status: StatusCode,
    content_type: &str,
    body: &Value,
) -> Option<String> {
    let mut problems = Vec::new();
    if status != StatusCode::BAD_REQUEST {
        problems.push(format!("状态码是 {status}，期望 400"));
    }
    if !content_type.starts_with("application/json") {
        problems.push(format!(
            "Content-Type 是 {content_type:?}，期望 application/json"
        ));
    }
    if !body.is_object() {
        problems.push(format!("响应体不是 JSON 对象: {body}"));
    } else {
        if !body["code"].is_number() {
            problems.push("响应体缺少数值型 code".to_string());
        }
        if !body["message"].is_string() {
            problems.push("响应体缺少字符串型 message".to_string());
        }
    }
    if problems.is_empty() {
        return None;
    }
    Some(format!(
        "{}（{}）\n    · {}\n    · 实际: {status} {content_type} {body}",
        endpoint,
        intent,
        problems.join("\n    · "),
    ))
}

/// **任何入参不合法，都必须回统一信封**
///
/// 判据落在**可观测的响应形状**上：遍历 OpenAPI 文档里的每个操作，
/// 对带请求体的发一个解析不了的 body、对带 uuid 路径参数的发一个非 UUID，
/// 两者都必须得到 `400` + `application/json` + `{code, message}`。
///
/// 为什么不写成"断言 36 处都改了"：那种断言是**自证**——
/// 漏掉的那一处根本不在清单里，测试照样全绿。而这里新增端点只要带了
/// 请求体或 uuid 路径参数，就自动进探针表；它若忘了用 `ApiJson` / `ApiPath`，
/// 响应会退回 `text/plain`，当场变红。
///
/// 三类探针分别对应 `map_rejection` 的不同分支：
/// - 坏 JSON → `JsonSyntaxError`
/// - 错的 Content-Type → `MissingJsonContentType`（即"415 并入 400"那条决定）
/// - 非 UUID 路径 → `FailedToDeserializePathParams`
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn every_bad_input_returns_unified_error_envelope() {
    let app = app().await;
    // **必须带合法令牌**：鉴权中间件先于提取器跑，无令牌时会拿到 401——
    // 那也是 JSON 信封，会让断言"因为错误的原因而通过"，等于没测提取器
    let token = admin_token(&app).await;

    let mut probes: Vec<BadInputProbe> = Vec::new();
    let mut body_probe_count = 0usize;
    let mut content_type_probe_count = 0usize;
    let mut path_probe_count = 0usize;

    for (method, path, op) in openapi_operations() {
        let uuids = uuid_path_params(&op);
        let concrete = substitute(&path, &uuids, VALID_UUID);
        let endpoint = format!("{method} {path}");

        // 探针一：带请求体的操作发解析不了的 JSON
        if !op["requestBody"].is_null() {
            body_probe_count += 1;
            probes.push(BadInputProbe {
                endpoint: endpoint.clone(),
                req: raw_request(
                    &method,
                    &concrete,
                    Some(&token),
                    Some("application/json"),
                    "{bad json",
                ),
                intent: "请求体解析失败",
            });
            // 探针二：带请求体的操作发对的 JSON 但错的 Content-Type
            //（原 axum 行为是 415 + text/plain，本版并入 400 + JSON）
            content_type_probe_count += 1;
            probes.push(BadInputProbe {
                endpoint: endpoint.clone(),
                req: raw_request(&method, &concrete, Some(&token), Some("text/plain"), "{}"),
                intent: "Content-Type 不是 application/json",
            });
        }

        // 探针三：带 uuid 路径参数的操作发非 UUID
        if !uuids.is_empty() {
            path_probe_count += 1;
            let broken = substitute(&path, &uuids, "not-a-uuid");
            probes.push(BadInputProbe {
                endpoint: endpoint.clone(),
                req: raw_request(&method, &broken, Some(&token), None, ""),
                intent: "路径参数不是 uuid",
            });
        }
    }

    // 探针表本身也要验：若 OpenAPI 结构变化导致一条都没解析出来，
    // 上面的循环会空转，断言全绿——那正是本用例最怕的"因为没测到而通过"
    assert!(
        body_probe_count >= 19,
        "从文档解析出的请求体探针只有 {body_probe_count} 条，探针表可能已失效"
    );
    assert!(
        content_type_probe_count >= 19,
        "从文档解析出的 Content-Type 探针只有 {content_type_probe_count} 条，探针表可能已失效"
    );
    assert!(
        path_probe_count >= 15,
        "从文档解析出的路径探针只有 {path_probe_count} 条，探针表可能已失效"
    );

    let mut violations = Vec::new();
    let probe_total = probes.len();
    for probe in probes {
        let BadInputProbe {
            endpoint,
            intent,
            req,
        } = probe;
        // `Request` 不是 `Clone`，所以探针按值消费；违规信息已提前取好，
        // 不需要把请求本身留在探针里
        let (status, content_type, body) = send_raw(&app, req).await;
        if let Some(violation) = envelope_violation(&endpoint, intent, status, &content_type, &body)
        {
            violations.push(violation);
        }
    }

    assert!(
        violations.is_empty(),
        "以下端点的入参错误没有走统一格式（共 {} 条探针）：\n\n{}",
        probe_total,
        violations.join("\n\n")
    );
}
