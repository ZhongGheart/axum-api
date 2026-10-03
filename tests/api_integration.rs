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
/// 确保迁移已应用，再让测试直接操作表
///
/// 迁移是由 `create_router` 触发的，所以**只有建过 app 的用例**才保证库里有表。
/// 直接 `INSERT audit_logs` 的用例此前只在全量跑时成立——
/// 字母序靠前的用例已经把库迁移过了。单跑其中一条就会撞
/// `relation "audit_logs" does not exist`。
/// 这类"靠别的用例先跑过"的前置依赖最难发现：全量绿，单独红。
async fn ensure_schema() {
    let _ = app().await;
}

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

// ──────────────────────────────────────────────
// 用户名 / 邮箱归一（v0.19.0）
// ──────────────────────────────────────────────

/// 三个写入入口（自助注册、管理员建号、管理员改号）都必须归一，且撞名回 409
///
/// 用户名不只是展示用：它是**登录键**，也是管理员在用户列表里辨认账号的依据。
/// 而 Postgres 的 `UNIQUE(username)` 是**大小写敏感**的——不归一的话
/// `Admin` 能与真 `admin` 并存，且自助注册一次就能造出来。
/// 管理员在列表上看到 `Admin` 无从判断它是不是真 admin，
/// 于是"给 admin 绑个角色"、"重置 admin 口令"这类操作会被引到伪造账号上。
///
/// 三个入口分开断言：只测其中一个，另外两个照样能漏。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn user_identities_are_normalized_on_every_write_path() {
    let app = app().await;
    let token = admin_token(&app).await;
    let suffix = unique("norm").to_lowercase();

    // ── 入口 1：自助注册 ──────────────────────────────────
    let noisy = format!("  MiXeD{suffix}  ");
    let canonical = format!("mixed{suffix}");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": noisy,
                "email": format!("  MiXeD{suffix}@Example.COM  "),
                "password": "normpass1A",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "注册应成功: {body}");
    assert_eq!(
        body["data"]["username"].as_str(),
        Some(canonical.as_str()),
        "注册时用户名应归一（trim + 小写）: {body}"
    );
    let registered_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    // 落库确为小写（不是"响应被改过、库里还是原样"）
    assert_eq!(
        email_in_db(&canonical).await,
        format!("mixed{suffix}@example.com"),
        "邮箱也应归一后落库"
    );
    assert!(
        !user_exists(&noisy).await,
        "原始写法 {noisy:?} 不应作为独立账号落库"
    );

    // 大小写变体撞名 → 409。**必须真的撞上同一个归一结果**：
    // 建的是 mixed_{suffix}，所以变体也得是 MIXED 前缀
    for variant in [
        canonical.to_uppercase(),
        format!("MiXeD{suffix}"),
        format!("mixed{suffix}  "),
    ] {
        let (status, body) = send(
            &app,
            request(
                "POST",
                "/api/auth/register",
                None,
                Some(json!({
                    "username": variant,
                    "email": format!("other{suffix}@example.com"),
                    "password": "normpass1A",
                })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "归一后同名应回 409（变体 {variant:?}）: {body}"
        );
    }

    // 邮箱同理：用户名换一个，邮箱只差大小写仍要 409
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": format!("othermails{suffix}"),
                "email": format!("MIXED{suffix}@EXAMPLE.COM"),
                "password": "normpass1A",
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "仅邮箱大小写不同也应回 409: {body}"
    );

    // ── 冒充内置账号：ADMIN / Admin 必须被挡 ────────────────
    //
    // 这是本项的**核心风险**，值得单独断言而不是顺带带过：
    // 不归一时它们与真 admin 并存，且管理员在用户列表上肉眼分不出来
    for impostor in ["ADMIN", "Admin", "aDmIn", " admin "] {
        let (status, body) = send(
            &app,
            request(
                "POST",
                "/api/auth/register",
                None,
                Some(json!({
                    "username": impostor,
                    "email": format!("impostor{suffix}@example.com"),
                    "password": "normpass1A",
                })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{impostor:?} 归一后等于 admin，不应能注册出冒充账号: {body}"
        );
    }
    // 邮箱也不能撞上真 admin
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": format!("mailimpostor{suffix}"),
                "email": "ADMIN@EXAMPLE.COM",
                "password": "normpass1A",
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "ADMIN@EXAMPLE.COM 归一后等于真 admin 邮箱，不应放行: {body}"
    );

    // ── 入口 2：管理员建号 ────────────────────────────────
    let managed_noisy = format!("  AdMin{suffix}  ");
    let managed_canonical = format!("admin{suffix}");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": managed_noisy,
                "email": format!("  AdMin{suffix}@Example.COM  "),
                "password": "user1234",
                "role": "user",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "管理员建号应成功: {body}");
    assert_eq!(
        body["data"]["username"].as_str(),
        Some(managed_canonical.as_str()),
        "管理员建号也应归一: {body}"
    );
    let managed_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();
    assert!(!user_exists(&managed_noisy).await);

    // 建号撞名（含大小写变体）→ 409
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": managed_canonical.to_uppercase(),
                "email": format!("dup{suffix}@example.com"),
                "password": "user1234",
                "role": "user",
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "管理员建号撞名应回 409: {body}"
    );

    // 管理员也不能建出 ADMIN 冒充账号
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": "ADMIN",
                "email": format!("adminimp{suffix}@example.com"),
                "password": "user1234",
                "role": "admin",
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "管理员不该能建出 ADMIN 冒充内置账号: {body}"
    );

    // ── 入口 3：管理员改号 ────────────────────────────────
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/users/{managed_id}"),
            Some(&token),
            Some(json!({
                "username": format!("  ReCased{suffix}  "),
                "email": format!("  ReCased{suffix}@Example.COM  "),
                "password": "user1234",
                "role": "user",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "管理员改号应成功: {body}");
    let recased = format!("recased{suffix}");
    assert_eq!(
        body["data"]["username"].as_str(),
        Some(recased.as_str()),
        "改号也应归一: {body}"
    );
    assert_eq!(
        email_in_db(&recased).await,
        format!("{recased}@example.com"),
        "改号时邮箱也应归一后落库"
    );

    // 改成别人的名字（含大小写变体）→ 409，且**不得**改坏原行
    for taken in [
        "ADMIN".to_string(),
        format!("MiXeD{suffix}"),
        canonical.to_uppercase(),
    ] {
        let (status, body) = send(
            &app,
            request(
                "PUT",
                &format!("/api/admin/users/{managed_id}"),
                Some(&token),
                Some(json!({
                    "username": taken,
                    "email": format!("{recased}@example.com"),
                    "password": "user1234",
                    "role": "user",
                })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "改成 {taken:?} 应回 409: {body}"
        );
        assert_eq!(
            email_in_db(&recased).await,
            format!("{recased}@example.com"),
            "冲突时不得留下半改的行"
        );
    }

    delete_user_via_api(&app, &token, managed_id).await;
    // 注册出来的账号没有角色，直接 SQL 删；留着会在共享库里攒下
    // 登录名含关键词的垃圾行，干扰后续按关键词查证的用例
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(registered_id)
        .execute(&pool().await)
        .await
        .expect("清理注册账号失败");
}

/// 登录对用户名与邮箱都**大小写不敏感**
///
/// 写入侧归一只做完一半：存下去的是小写，若登录仍按原样查库，
/// 用户改成习惯的大小写后就登不进来——用户视角就是"我明明注册成功了"。
/// 用户名和邮箱两条查询路径都要验，因为登录框两者都接受。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn login_accepts_any_casing_of_username_and_email() {
    let app = app().await;
    let suffix = unique("caselogin").to_lowercase();
    let username = format!("mixed{suffix}");
    let email = format!("mixed{suffix}@example.com");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": format!("  MiXeD{suffix}  "),
                "email": format!("  MiXeD{suffix}@Example.COM  "),
                "password": "caselogin1A",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "注册应成功: {body}");
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    // 用户名的各种大小写
    for variant in [
        username.clone(),
        username.to_uppercase(),
        format!("MiXeD{suffix}"),
        format!("  MIXED{suffix}  "),
    ] {
        let (status, body) = login(&app, &variant, "caselogin1A").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "用户名变体 {variant:?} 应能登录: {body}"
        );
    }

    // 邮箱的各种大小写
    for variant in [
        email.clone(),
        email.to_uppercase(),
        format!("MiXeD{suffix}@Example.COM"),
        format!("  MIXED{suffix}@EXAMPLE.COM  "),
    ] {
        let (status, body) = login(&app, &variant, "caselogin1A").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "邮箱变体 {variant:?} 应能登录: {body}"
        );
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool().await)
        .await
        .expect("清理账号失败");
}

/// 数据库层的函数唯一索引：仅大小写不同的行直接写库也会被拒
///
/// 应用侧归一是第一道防线，但**任何绕过应用的写库路径**（手工 SQL、
/// 导数据、将来某个漏了归一的新入口）都得被挡住，否则就是等到
/// "用户列表里出现两个肉眼一样的账号"才被发现。
///
/// 顺带钉住一件事：撞的必须是 `*_lower_key` 而不是迁移 001 的 `*_key`，
/// 因为完全相同的名字先撞旧约束、只有大小写不同才撞新的那个。
/// 仓库层据此翻译 409——只认旧名字的话这条会变成 500。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn the_database_rejects_identities_differing_only_in_case() {
    let app = app().await;
    let suffix = unique("dbcase").to_lowercase();
    let username = format!("dbcase{suffix}");
    let email = format!("{username}@example.com");

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": username,
                "email": email,
                "password": "dbcasepass1A",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "注册应成功: {body}");
    let user_id: uuid::Uuid = body["data"]["id"].as_str().unwrap().parse().unwrap();

    // 用户名仅大小写不同
    let err =
        sqlx::query("INSERT INTO users (username, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(username.to_uppercase())
            .bind(format!("u{suffix}@example.com"))
            .execute(&pool().await)
            .await
            .expect_err("仅大小写不同的用户名应在数据库层被拒");
    let msg = err.to_string();
    assert!(
        msg.contains("users_username_lower_key"),
        "应撞函数唯一索引，实际: {msg}"
    );

    // 邮箱仅大小写不同
    let err =
        sqlx::query("INSERT INTO users (username, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(format!("e{suffix}"))
            .bind(email.to_uppercase())
            .execute(&pool().await)
            .await
            .expect_err("仅大小写不同的邮箱应在数据库层被拒");
    let msg = err.to_string();
    assert!(
        msg.contains("users_email_lower_key"),
        "应撞函数唯一索引，实际: {msg}"
    );

    // 仓库层必须把这个约束名翻成 409，而不是 500
    //
    // 直接调仓储而不是绕 HTTP：走 HTTP 时应用侧的查重会先一步挡住，
    // 永远到不了这条路径——于是"只认 users_username_key"这个缺陷
    // 就能一路绿灯发布，直到某个绕过归一的入口撞上它才在生产上 500。
    // 这条断言存在的意义就是**不让那条路径保持不可见**。
    let repo = axum_api::repository::user::UserRepository::new(pool().await);
    let err = repo
        .create(
            uuid::Uuid::new_v4(),
            &username.to_uppercase(),
            &format!("repo{suffix}@example.com"),
            "x",
            false,
        )
        .await
        .expect_err("仅大小写不同的用户名在仓库层也应回冲突，而不是内部错误");
    assert!(
        matches!(err, axum_api::error::AppError::Conflict(_)),
        "仓库层应把 users_username_lower_key 翻成 409，实际: {err:?}"
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool().await)
        .await
        .expect("清理账号失败");
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
    //
    // 必须**翻页找**：列表默认每页 10 条，排序是 created_at ASC，
    // 刚建的角色排在末尾，只取首页找不到它——那是分页语义，不是 bug。
    // 但也不能只写死 `page_size=200` 就完事：`page_size` 上限就是 200，
    // 而测试库的角色数会随 `operator_with_codes` 的残留一路累积
    // （实测已过 200），于是"取 200 条"同样找不到它。
    // 这不是分页的错，是"假设数据够少"的错——翻页找才是与数据量无关的写法。
    let from_list = find_role_row_by_id(&app, &token, &role_id.to_string())
        .await
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
    let (tok, op_role_id, op_uid) =
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    ensure_schema().await;
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
    ensure_schema().await;
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

    let outcome = repo.delete_older_than(cutoff, 10, 5).await.unwrap();
    assert_eq!(outcome.deleted, 3, "只应删掉 3 条过期日志");
    // 3 < 批大小 10，说明这一批就把过期行删尽了：不是"撞上限收手"
    assert!(
        !outcome.hit_batch_limit,
        "删到不足一批即说明清干净了，不该报撞上限"
    );

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
    ensure_schema().await;
    let repo = AuditLogRepository::new(pool().await);
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(1);

    let mut ids = Vec::new();
    for _ in 0..25 {
        ids.push(insert_audit_log(now - chrono::Duration::days(5)).await);
    }

    let outcome = repo.delete_older_than(cutoff, 10, 2).await.unwrap();
    assert_eq!(
        outcome.deleted, 20,
        "两批 × 每批 10 条，不应超出 max_batches"
    );
    // 每一批都恰好删满 10 且用完了 2 批预算 —— 说明还有 5 条没轮到。
    // 这个区别必须报出来，否则"还有更多过期数据没清"会被当成"已经清干净"。
    assert!(
        outcome.hit_batch_limit,
        "两批都删满且预算用尽，应报撞上限提前收手"
    );

    let p = pool().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(count, 5, "应正好剩 5 条待下一轮清理");

    // 放开批次上限后应能删干净
    let outcome = repo.delete_older_than(cutoff, 10, 5).await.unwrap();
    assert_eq!(outcome.deleted, 5);
    assert!(
        !outcome.hit_batch_limit,
        "最后一批只剩 5 条、不满批，说明这次是真删干净了"
    );
    assert!(!audit_log_exists(ids[0]).await, "清理完后不应有残留");

    sqlx::query("DELETE FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&p)
        .await
        .unwrap();
}

/// [`operator_with_codes`] 造出的角色与账号一律以此开头
///
/// 守卫靠它圈定残留范围。**带这个前缀，而不是扫"所有 `_role_` 结尾的名字"**：
/// `grantee_role_*`、`strong_role_*`、`tmp_holder_role_*` 都是别的夹具造的，
/// 混进来会让守卫要么长期误报、要么被人加一条豁免——两者都等于没有守卫。
const OPERATOR_FIXTURE_PREFIX: &str = "opf_";

/// 造一个**不是 admin**、只持有指定权限码的操作员，返回 (token, role_id, user_id)
///
/// 这是 PR-3 的核心夹具。撤掉 `require_role("admin")` 之后，"能进管理区"
/// 完全由这些码决定，所以提权面**必须用持有部分码的非 admin 才能测出来**——
/// 拿 admin 当夹具会把所有"应被拒"的断言都测成"当然通过"。
///
/// 造出来的角色与账号都带 [`OPERATOR_FIXTURE_PREFIX`] 前缀，理由见该常量。
///
/// **每个调用点都必须配一次 [`cleanup_operator]`**：这条夹具被 19 个用例共用，
/// 漏一处就是库里多一个角色加一个账号，而共享库会被一轮轮堆肥。
async fn operator_with_codes(
    app: &Router,
    admin_tok: &str,
    prefix: &str,
    codes: &[&str],
) -> (String, uuid::Uuid, uuid::Uuid) {
    let role_name = unique(&format!("{OPERATOR_FIXTURE_PREFIX}{prefix}_role"));
    let role_id = create_role_via_api(app, admin_tok, &role_name).await;

    let mut menu_ids = Vec::new();
    for code in codes {
        menu_ids.push(menu_id_of(code).await);
    }
    let (status, body) = assign_menus(app, admin_tok, role_id, &menu_ids).await;
    assert_eq!(status, StatusCode::OK, "给测试角色授权失败: {body}");

    let username = unique(&format!("{OPERATOR_FIXTURE_PREFIX}{prefix}_user"));
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

/// 收回 [`operator_with_codes`] 造出来的角色与账号
///
/// **顺序是先删用户、后删角色**，实测反序会被挡回：
/// `DELETE /api/admin/roles/{id}` 在"仍有 N 个用户使用该角色"时回 400
/// 「请先调整这些用户的角色」（`src/controller/role.rs`）。那条拒绝是**有意设计**——
/// `user_roles` 的 `ON DELETE CASCADE` 会**静默**剥掉这些用户的角色，
/// 让人变成"没有任何角色"的用户而不自知。所以清理要顺着它的意思来，
/// 而不是绕开它（先 `DELETE FROM roles` 让级联生效）。
///
/// 这里走 API，而不像 [`cleanup_holder`] 那样走 SQL：本夹具的角色挂的是
/// `system:user:list` 这类**真实**权限码，admin 按种子持有全部，授权下界
/// （能授予的 ⊆ 已持有的）自然放行。`cleanup_holder` 的角色挂的是
/// `tmp:*:priv:*` 一次性专属码，admin 按设计不持有，才只能走 SQL。
/// 两者形态不同不是随意选择，是被各自的授权状态决定的。
async fn cleanup_operator(app: &Router, admin_tok: &str, user_id: uuid::Uuid, role_id: uuid::Uuid) {
    delete_user_via_api(app, admin_tok, user_id).await;
    let (status, body) = delete_role(app, admin_tok, role_id).await;
    assert_eq!(status, StatusCode::OK, "清理测试操作员角色失败: {body}");
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
    let (tok, op_role_id, op_uid) =
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let (tok, op_role_id, uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, uid, op_role_id).await;
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
    let (tok, op_role_id, uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, uid, op_role_id).await;
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
    let (tok, role_id, op_uid) =
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
    cleanup_operator(&app, &admin_tok, op_uid, role_id).await;
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
    let (tok, role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, role_id).await;
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
async fn granted_temp_button(app: &Router, admin_tok: &str, code_prefix: &str) -> TempCodeFixture {
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

    TempCodeFixture {
        dir_id,
        btn_id,
        code,
        holder_tok,
        holder_role,
        holder_uid,
    }
}

/// `granted_temp_button` 造出来的四样东西：临时目录、临时按钮、专属码、持有者
///
/// 用具名结构体而不是匿名元组：这份账本此前漏了角色与持有者账号，
/// 而元组允许调用方用 `_holder_uid` 把它们随手丢掉——一丢就再也找不回来。
struct TempCodeFixture {
    dir_id: uuid::Uuid,
    btn_id: uuid::Uuid,
    code: String,
    holder_tok: String,
    holder_role: uuid::Uuid,
    holder_uid: uuid::Uuid,
}

/// 清理 `granted_temp_button` 造的持有者角色与账号
///
/// **必须走 SQL，不能走 API**——这正是本轮实测的结论（探针那边同源）：
/// 持有者角色上挂着 `tmp:*:priv:*` 这种一次性专属码，而 admin 按设计不持有它
/// （种子是"只授权新建行"，管理员在菜单页撤销的授权不该被下次启动悄悄恢复）。
/// 于是 `DELETE /api/admin/roles/{id}` 与 `DELETE /api/admin/users/{id}`
/// 都会被授权下界挡回 403/400——**用 API 清理自己造的夹具会被自己测的守卫锁死**。
///
/// 降权方向（收回授权）不设限，但走 API 要多两跳且仍可能被"仍有用户持有该角色"
/// 挡下；这里直接按外键级联删行，是测试夹具自己的账本，不该再考验被测逻辑。
async fn cleanup_holder(holder: &TempCodeFixture) {
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(holder.holder_uid)
        .execute(&pool().await)
        .await
        .expect("清理持有者账号失败");
    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(holder.holder_role)
        .execute(&pool().await)
        .await
        .expect("清理持有者角色失败");
}

/// 权限码夹具跑完后不该留下持有者——把"漏清理"从惯例变成红灯
///
/// 上一轮实测（`git stash` 验过不是本轮引入的回归）：`granted_temp_button`
/// 每跑一次就把一个持有者角色和一个持有者账号留在库里——临时菜单目录有
/// `cleanup_temp_menu_dir` 收拾，这两个没有——而 125 个用例**全绿**。
/// 测试不检查自己留下的垃圾，就永远发现不了自己在漏：共享库会被一轮轮堆肥，
/// 分页类断言的噪声基线也随之抬高。
///
/// **v0.19.0 把范围补上了 `operator_with_codes` 那一支**——此前这里只圈
/// `granted_temp_button`，注释里写着"那是另一笔账、另一个版本的活"。
/// 那笔账现在还：19 个调用点没有一处清理，实测累积 212 个操作员角色、
/// 424 个账号（整库 270 角色 / 424 用户，即绝大多数都是它）。
/// 补法是给夹具加 [`OPERATOR_FIXTURE_PREFIX`] 前缀 + 19 处配 [`cleanup_operator`]，
/// 这样本条才能在**不动别的夹具**的前提下把范围收干净。
/// 注意顺序：先有前缀与清理，最后才把范围放大——反过来本条会长期红，
/// 而一个长期红的守卫等于没有守卫。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn the_permission_code_fixtures_leave_no_holder_behind() {
    let p = pool().await;

    /// 每支夹具一行：`(夹具名, 角色名 LIKE, 账号名 LIKE)`
    ///
    /// LIKE 里的 `_` 要转义成 `\_`，否则它匹配任意单字符——
    /// 于是 `tmp_holder_role_%` 会连 `tmpXholderYroleZ...` 一起捞进来。
    const FIXTURES: &[(&str, &str, &str)] = &[
        (
            "granted_temp_button 持有者",
            r"tmp\_holder\_role\_%",
            r"tmp\_holder\_user\_%",
        ),
        ("operator_with_codes 操作员", "opf\\_%", "opf\\_%"),
    ];

    let mut leaked: Vec<String> = Vec::new();
    for (fixture, role_pattern, user_pattern) in FIXTURES {
        let roles: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT name FROM roles WHERE name LIKE '{role_pattern}'"
        ))
        .fetch_all(&p)
        .await
        .unwrap_or_else(|e| panic!("扫描 {fixture} 的残留角色失败: {e}"));
        let users: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT username FROM users WHERE username LIKE '{user_pattern}'"
        ))
        .fetch_all(&p)
        .await
        .unwrap_or_else(|e| panic!("扫描 {fixture} 的残留账号失败: {e}"));
        leaked.extend(roles.into_iter().map(|n| format!("{fixture}: 角色 {n}")));
        leaked.extend(users.into_iter().map(|n| format!("{fixture}: 账号 {n}")));
    }

    assert!(
        leaked.is_empty(),
        "权限码夹具留下了 {} 条残留（共享库会被一轮轮堆肥）:\n  - {}",
        leaked.len(),
        leaked.join("\n  - ")
    );
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
                 UNION
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

/// 把 [id] 及其整棵子树里的所有菜单摘成根节点
///
/// 专治"清理助手在环上把自己挂死"：`cleanup_temp_menu_dir` 要算子树
///（SQL 递归），而 v0.17.0 的一批用例会**故意造环**。清理前先把环剪开。
/// 递归用 `UNION` 去重：万一目标本身就在环里，也只是转一圈就停。
async fn flatten_menus_to_roots(id: uuid::Uuid) {
    sqlx::query(
        "WITH RECURSIVE subtree AS (
             SELECT id FROM menus WHERE id = $1
             UNION
             SELECT m.id FROM menus m JOIN subtree s ON m.parent_id = s.id
         )
         UPDATE menus SET parent_id = NULL
         WHERE id IN (SELECT id FROM subtree) AND parent_id IS NOT NULL",
    )
    .bind(id)
    .execute(&pool().await)
    .await
    .expect("剪开菜单环失败");
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
    let holder = granted_temp_button(&app, &admin_tok, "tmp:restore").await;
    let (dir_id, btn_id, code, holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
    );

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
    assert_eq!(cleared_by, Some(holder.holder_uid), "应记下清空者");

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
    cleanup_holder(&holder).await;
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
    let holder = granted_temp_button(&app, &admin_tok, "tmp:clear").await;
    let (dir_id, btn_id, code, _holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
    );

    // 操作员只持 menu:update，不持那个一次性码
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_holder(&holder).await;
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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

    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
}

/// 恢复是"撤销我自己的误操作"，不是"接管别人的清空"
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn only_the_clearing_user_can_restore_a_permission_code() {
    use axum_api::model::permission;

    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let holder = granted_temp_button(&app, &admin_tok, "tmp:owner").await;
    let (dir_id, btn_id, code, holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
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

    // 另一个同样持 menu:update 的操作员来恢复 —— 不是他清的
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_holder(&holder).await;
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
}

/// 菜单树要告诉前端"这个按钮的码可以恢复"，否则恢复入口无从发现
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn the_menu_tree_reports_a_restorable_permission_code() {
    let app = app().await;
    let admin_tok = admin_token(&app).await;
    let holder = granted_temp_button(&app, &admin_tok, "tmp:tree").await;
    let (dir_id, btn_id, code, holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
    );

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
    cleanup_holder(&holder).await;
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
    let holder = granted_temp_button(&app, &admin_tok, "tmp:del").await;
    let (dir_id, btn_id, code, holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
    );

    // 操作员只持 menu:delete，不持那个一次性码，也没有 menu:grant
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_holder(&holder).await;
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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

    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let holder = granted_temp_button(&app, &admin_tok, "tmp:delcascade").await;
    let (dir_id, btn_id, code, holder_tok) = (
        holder.dir_id,
        holder.btn_id,
        holder.code.clone(),
        holder.holder_tok.clone(),
    );

    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_holder(&holder).await;
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let (tok, op_role_id, uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, uid, op_role_id).await;
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
    let (tok, op_role_id, op_uid) =
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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
    let (tok, op_role_id, op_uid) = operator_with_codes(
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
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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

    let (tok, op_role_id, op_uid) = operator_with_codes(
        &app,
        &admin_tok,
        "delrole_empty_op",
        &[permission::ROLE_DELETE],
    )
    .await;

    let (status, body) = delete_role(&app, &tok, empty_id).await;
    assert_eq!(status, StatusCode::OK, "无码角色不应被天花板拦下: {body}");
    assert!(!role_still_exists(empty_id).await, "角色应已被删除");
    cleanup_operator(&app, &admin_tok, op_uid, op_role_id).await;
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

/// 翻页取回全部角色 id（按 `page_size` 逐页取，直到取空）
///
/// 角色列表**没有 keyword 参数**（`RoleListParams` 只有 page / page_size），
/// 而 `page_size` 上限是 200。所以"一次取全量"这件事在角色总数超过 200 时
/// **根本无法表达**——这正是下面那条用例曾经变红的原因。
async fn all_role_ids(app: &Router, token: &str, page_size: i64) -> Vec<String> {
    let mut ids = vec![];
    let mut page = 1;
    loop {
        let (status, body) = send(
            app,
            request(
                "GET",
                &format!("/api/admin/roles?page={page}&page_size={page_size}"),
                Some(token),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "第 {page} 页取角色失败: {body}");
        let items = body["data"]["items"]
            .as_array()
            .expect("data.items 应为数组");
        if items.is_empty() {
            break;
        }
        for r in items {
            ids.push(r["id"].as_str().unwrap_or_default().to_string());
        }
        page += 1;
        assert!(page < 500, "翻页没有收敛，可能陷入死循环");
    }
    ids
}

/// 在分页列表里逐页找出指定 id 的那一行
///
/// 排序是 `created_at ASC`，刚建的角色**排在末尾**，
/// 所以只取首页必然找不到它；而写死 `page_size=200` 在角色总数超过 200 时
/// 也找不到——那不是分页语义，是"假设数据够少"。
async fn find_role_row_by_id(app: &Router, token: &str, id: &str) -> Option<Value> {
    let mut page = 1;
    loop {
        let (status, body) = send(
            app,
            request(
                "GET",
                &format!("/api/admin/roles?page={page}&page_size=200"),
                Some(token),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "第 {page} 页取角色失败: {body}");
        let items = body["data"]["items"]
            .as_array()
            .expect("data.items 应为数组");
        if let Some(hit) = items.iter().find(|r| r["id"].as_str() == Some(id)) {
            return Some(hit.clone());
        }
        if items.is_empty() {
            return None;
        }
        page += 1;
        assert!(page < 500, "翻页没有收敛，可能陷入死循环");
    }
}

/// 角色列表返回分页对象，且**用两种页长翻页取回的集合完全一致**
///
/// ## 为什么不再断言"一次取全量"
///
/// 原来这条用例取 `page_size=200` 当"一页装得下全部"，再断言
/// `total == items.len()`。可接口的 `page_size` 上限就是 200——
/// 一旦库里角色超过 200 条（`operator_with_codes` 每轮漏一批，测试库长期累积），
/// 这个前提本身就无法成立，`total == items.len()` 于是变成一句**错的话**：
/// 它断言的不是分页正确，而是"数据够少"。
///
/// 值得注意的是，这个坑与用例上方那段注释**自相矛盾**：
/// 那段特意强调"别写死条数，会被脏数据永久打红"，
/// 而 `page_size=200` 正是同一种写死，只是换了个字段名。
///
/// 现在断言的是分页响应的真正不变量：
/// 1. `items.len() == min(total, page_size)` —— 该给多少给多少，不多不少
/// 2. 页长 2 与页长 200 翻页取回的**集合相同** —— 既不重也不漏
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn role_list_pages_over_the_same_set_as_one_big_page() {
    let app = app().await;
    let token = admin_token(&app).await;

    let mut created = vec![];
    for _ in 0..5 {
        created.push(create_role_via_api(&app, &token, &unique("pgrole")).await);
    }

    // ★ 关键：必须由测试自己造出 `created_at` 并列。
    //
    // 排序键只有 `created_at ASC` 时，同一时刻的行之间次序由数据库自行决定，
    // 翻页就会跨页重复或漏行。若不主动制造并列，这条守卫能不能变红
    // 完全取决于"库里现有数据的 created_at 恰好不并列"——那是运气，不是断言。
    // （先前手工把全表 created_at 改成同一值才发现它会红，就是这个坑。）
    //
    // 所以这里把本用例新建的角色统一按同一时刻落库，让"稳定排序"成为刚需。
    // 取 5 个而不是 3 个：`created_at=2020` 让它们排在全表最前，页长 2 时
    // 并列组会横跨第 1/2/3 页之间的**两个**页边界——只要组内次序有抖动，
    // 翻页结果就一定会出现重复或缺失，而不是靠运气侥幸通过。
    sqlx::query("UPDATE roles SET created_at = '2020-01-01T00:00:00Z' WHERE id = ANY($1)")
        .bind(created.clone())
        .execute(&pool().await)
        .await
        .expect("把新建角色的 created_at 对齐失败");

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
    let total = whole["data"]["total"].as_i64().expect("缺少 total");
    let on_page = whole["data"]["items"]
        .as_array()
        .expect("data.items 应为数组")
        .len() as i64;
    assert_eq!(
        on_page,
        total.min(200),
        "单页条数必须是 min(total, page_size)：多了会跨页串行，少了说明漏读。total={total}"
    );
    assert!(whole["data"]["total_pages"].as_i64().unwrap_or(0) >= 1);

    // 用两种页长各自翻完，集合必须一致：既不重也不漏
    let paged_small = all_role_ids(&app, &token, 2).await;
    let paged_big = all_role_ids(&app, &token, 200).await;

    // 采集完就清掉本用例造的角色，**然后**才做断言。
    // 否则一旦下面某条断言 panic，清理永远不会执行——而断言恰恰会在
    // "分页真的有 bug"时失败，那时留下的脏数据 created_at=2020 排在全表最前，
    // 会持续污染后续所有用例（注入验证时确实漏下过 17 个角色）。
    let created_strs: Vec<String> = created.iter().map(|id| id.to_string()).collect();
    for id in &created {
        let _ = delete_role(&app, &token, *id).await;
    }

    // 先跟 total 对账。若翻页漏读，两种页长可能**同样**漏掉同一批，
    // 集合比对就会一起假绿——所以"总数吻合"必须独立成立。
    assert_eq!(
        paged_small.len() as i64,
        total,
        "按页长 2 翻完只取回 {}/{} 条，说明分页漏读，两种页长比对会一起假绿",
        paged_small.len(),
        total
    );

    let mut sorted_paged = paged_small.clone();
    sorted_paged.sort();
    sorted_paged.dedup();
    assert_eq!(
        sorted_paged.len(),
        paged_small.len(),
        "翻页取回了重复的角色，说明分页缺稳定排序: {paged_small:?}"
    );

    let mut sorted_big = paged_big.clone();
    sorted_big.sort();
    assert_eq!(sorted_paged, sorted_big, "不同页长翻页取回的角色集合不一致");

    // 刚建的五个角色必须真的出现在结果里（否则上面两条可能都在比空集）。
    // 判定只看归属，不看返回位置——注入故障时它们可能被排到别的页，
    // 位置本身就是不可靠的。
    for id in &created_strs {
        assert!(
            sorted_paged.contains(id),
            "刚创建的角色 {id} 不在翻页结果里，说明集合比对在比一个不含新增项的旧快照"
        );
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

            // 属性块整体在结构体之上，中间可以夹注释和别的属性
            // （`RoleListParams` 的 `#[serde(deny_unknown_fields)]` 上面隔着
            //  `#[derive]`、下面隔着 7 行解释 utoipa 的注释和
            //  `#[into_params(parameter_in = Query)]`）。所以只取紧邻上一行会误报，
            //  正确做法是向上走到**连续**的 attribute/注释块结束再判断。
            let mut block: Vec<&str> = vec![];
            for line in source[..abs_start].lines().rev() {
                let t = line.trim();
                if t.starts_with('#') || t.starts_with("//") {
                    block.push(t);
                    continue;
                }
                break;
            }
            if !block.contains(&"#[serde(deny_unknown_fields)]") {
                // 报出整个块，让失败当场可读，而不是只说"少了某个属性"
                let seen = if block.is_empty() {
                    "（结构体上方没有任何属性）".to_string()
                } else {
                    block.iter().rev().cloned().collect::<Vec<_>>().join(" / ")
                };
                offenders.push(format!(
                    "{file}::{name} 缺少 #[serde(deny_unknown_fields)]（属性块：{seen}）"
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

/// 非 UUID 路径参数的探针值（如 `GET /api/dict/{code}/items` 的 `code`）
///
/// 取一个一定不存在的字典码：本守卫要的是"路由和处理函数都跑到了"，
/// 而不是"这个码真的有数据"。
const NON_UUID_PARAM_PROBE: &str = "no_such_code_probe";

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

/// 该操作路径模板里**全部**路径参数的参数名（含非 uuid 的）
///
/// 与 [`uuid_path_params`] 互补：后者只挑 `format: uuid` 的，
/// 剩下的（当前只有 `GET /api/dict/{code}/items` 的 `code`）要靠
/// [`NON_UUID_PARAM_PROBE`] 填上，否则路径模板会带着 `{code}` 字面量发出去，
/// 打到路由上得到 404 —— 一个"因为没匹配到路由而通过"的假绿。
fn all_path_params(op: &Value) -> Vec<String> {
    op["parameters"]
        .as_array()
        .map(|params| {
            params
                .iter()
                .filter(|p| p["in"] == "path")
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 该操作**必填**的 query 参数名
///
/// `GET /api/admin/dict/items` 的 `dict_type_id` 是必填的：不补上它，
/// 查询提取器会先回 400，探针根本走不到处理函数，于是"这个端点能跑"
/// 依然是句没验证过的话。判据取自文档的 `required`，不写死清单。
fn required_query_params(op: &Value) -> Vec<String> {
    op["parameters"]
        .as_array()
        .map(|params| {
            params
                .iter()
                .filter(|p| p["in"] == "query" && p["required"] == true)
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 由文档派生出一条**能走到处理函数**的具体请求路径
///
/// uuid 路径参数填 [`VALID_UUID`]（不存在的行，于是走 404 而不是误删真数据），
/// 非 uuid 的填 [`NON_UUID_PARAM_PROBE`]，必填 query 参数也补上。
fn concrete_path(path: &str, op: &Value) -> String {
    let uuids = uuid_path_params(op);
    let mut concrete = substitute(path, &uuids, VALID_UUID);
    let others: Vec<String> = all_path_params(op)
        .into_iter()
        .filter(|n| !uuids.contains(n))
        .collect();
    concrete = substitute(&concrete, &others, NON_UUID_PARAM_PROBE);
    for name in required_query_params(op) {
        concrete.push_str(&format!("?{name}={VALID_UUID}"));
    }
    concrete
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

// ──────────────────────────────────────────────
// v0.13.0：审计要能回答"改了什么"
// ──────────────────────────────────────────────
//
// 此前 `audit_logs.result` 对**所有写操作恒为空**，实测：
//
//   DELETE /api/admin/roles/<uuid>       params 空 | result 空
//   PUT    /api/admin/roles/<uuid>/menus params 空 | result 空
//
// 于是事后追溯只能拿到"某时刻有人删了个 UUID"——
// 角色名只存在 `roles` 行里，行删掉就没了；授权授予更是连"授了哪些码"都查不到。
//
// 本节用例**逐个走完所有写入口**并断言审计里能读到该读的事实：
// 资源名、权限码差异、口令重置的对象。三条不可省的性质：
//
// 1. **删除之后名字仍在**——这是本版存在的理由
// 2. **口令一个字都不入库**——摘要只能由 handler 显式声明，
//    自动记录请求体会把 `password` 写进长期表
// 3. **失败的写操作不留摘要**——否则审计会谎报"已授予/已删除"

/// 轮询审计表，直到找到 `(method, path)` 下**含 marker** 的那条摘要
///
/// 中间件是 `tokio::spawn` 异步写的，读完响应时那一条可能还没落库。
/// 直接查一次会偶发失败——测试自己说谎比功能缺陷更难查。
async fn wait_for_audit_result(method: &str, path: &str, marker: &str) -> String {
    let sql = "SELECT result FROM audit_logs \
                WHERE method = $1 AND path = $2 AND result LIKE '%' || $3 || '%' \
                ORDER BY created_at DESC, id DESC LIMIT 1";
    for _ in 0..40 {
        let found: Option<(String,)> = sqlx::query_as(sql)
            .bind(method)
            .bind(path)
            .bind(marker)
            .fetch_optional(&pool().await)
            .await
            .expect("查询审计摘要失败");
        if let Some((result,)) = found {
            return result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("审计里查不到 {method} {path} 中含有「{marker}」的摘要");
}

/// `audit_logs` 全表中含有给定片段的行数（`params` 与 `result` 都查）
///
/// 用来证明**口令没有落进审计**。只查 `result` 是不够的：
/// `params` 列存的是查询串，一旦有人改成记请求体，秘密就从这里漏出去。
async fn audit_rows_containing(needle: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_logs \
         WHERE (params IS NOT NULL AND params LIKE '%' || $1 || '%') \
            OR (result IS NOT NULL AND result LIKE '%' || $1 || '%')",
    )
    .bind(needle)
    .fetch_one(&pool().await)
    .await
    .expect("统计审计行失败")
}

/// 写操作审计摘要的承重测试
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn every_write_operation_leaves_an_answerable_change_summary() {
    let app = app().await;
    let token = admin_token(&app).await;
    let tag = unique("audited");

    // ── 1. 建角色 → 摘要里有角色名 ──────────────────────────
    let role_name = format!("{tag}_role");
    let role_id = create_role_via_api(&app, &token, &role_name).await;
    wait_for_audit_result("POST", "/api/admin/roles", &role_name).await;

    // ── 2. 改角色名 → **新旧两个名字都要在** ─────────────────
    // 事后只看到新名字，仍然答不出"这个角色原来叫什么"
    let renamed = format!("{role_name}_v2");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/roles/{role_id}"),
            Some(&token),
            Some(json!({ "name": renamed, "description": "改名后的描述" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改角色名失败: {body}");
    let summary =
        wait_for_audit_result("PUT", &format!("/api/admin/roles/{role_id}"), &renamed).await;
    assert!(
        summary.contains(&role_name),
        "改名审计必须同时含旧名，否则答不出「原来叫什么」: {summary}"
    );

    // ── 3. 授权：授予 → 审计记下授了哪些码 ────────────────────
    let user_list = menu_id_of(axum_api::model::permission::USER_LIST).await;
    let role_list = menu_id_of(axum_api::model::permission::ROLE_LIST).await;
    let grant_path = format!("/api/admin/roles/{role_id}/menus");
    let (status, body) = assign_menus(&app, &token, role_id, &[user_list, role_list]).await;
    assert_eq!(status, StatusCode::OK, "首次授权应成功: {body}");
    let summary =
        wait_for_audit_result("PUT", &grant_path, axum_api::model::permission::USER_LIST).await;
    assert!(
        summary.contains(axum_api::model::permission::ROLE_LIST),
        "授予审计必须列出本次授出的**全部**权限码: {summary}"
    );
    assert!(
        summary.contains(&renamed),
        "授权审计必须点名是哪个角色，否则事后无法定位: {summary}"
    );

    // ── 4. 撤权：全量替换 → 审计记下**撤了哪些**码 ─────────────
    // 这是本版最关键的一条：撤销恰恰是事后追溯最想知道的那一半，
    // 而只记提交上来的集合根本答不出来
    let (status, body) = assign_menus(&app, &token, role_id, &[user_list]).await;
    assert_eq!(status, StatusCode::OK, "重新授权应成功: {body}");
    let summary = wait_for_audit_result("PUT", &grant_path, "撤销权限码").await;
    assert!(
        summary.contains(axum_api::model::permission::ROLE_LIST),
        "撤权审计必须指名被撤销的权限码: {summary}"
    );
    // 全量替换语义下，留在集合里的码不是"本次变更"
    assert!(
        !summary.contains(&format!(
            "撤销权限码 {}",
            axum_api::model::permission::USER_LIST
        )),
        "仍在提交集合中的码不该被记成撤销: {summary}"
    );

    // ── 5. 建按钮菜单 → 摘要含声明的权限码 ────────────────────
    let code_a = format!("{tag}:probe:a");
    let menu_name = format!("{tag}_按钮");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(json!({
                "name": menu_name,
                "type": "button",
                "permission": code_a,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新建按钮菜单失败: {body}");
    let menu_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("新建菜单响应里没有 id: {body}"));
    let menu_path = format!("/api/admin/menus/{menu_id}");
    wait_for_audit_result("POST", "/api/admin/menus", &code_a).await;

    // 刻意**不测** `code_a → code_b` 的改码：接口层走不通。
    // `update_menu` 要求调用者已持有目标码，而目标码已存在时又撞唯一索引 409，
    // 两条路互相堵死（详见 `audit::permission_change` 的说明）。
    // 那条摘要分支由该函数的单测承重。

    // ── 6. 清空权限码 → 记成"改为无"，且**旧码必须留痕** ────────
    // 按钮清空后 `menus.permission` 就是 NULL，旧码只存在于审计与恢复槽位里。
    // 摘要若不留旧码，事后就答不出"这个按钮刚才管的是哪个权限"
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &menu_path,
            Some(&token),
            Some(json!({ "permission": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清空权限码失败: {body}");
    let summary = wait_for_audit_result("PUT", &menu_path, "改为 \"无\"").await;
    assert!(
        summary.contains(&code_a),
        "清空审计必须保留被清掉的码: {summary}"
    );

    // ── 7. 恢复 → 摘要含恢复回来的码 ─────────────────────────
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("{menu_path}/restore-permission"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "恢复权限码失败: {body}");
    wait_for_audit_result("POST", &format!("{menu_path}/restore-permission"), &code_a).await;

    // ── 8. 删菜单 → 名字必须留存在审计里 ─────────────────────
    let (status, body) = send(&app, request("DELETE", &menu_path, Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK, "删除菜单失败: {body}");
    wait_for_audit_result("DELETE", &menu_path, &menu_name).await;
    assert!(
        !menu_still_exists(menu_id).await,
        "菜单应已删除（否则这条审计说明的是一次没发生的删除）"
    );

    // ── 10. 建用户 → 摘要含用户名与角色 ───────────────────────
    let username = format!("{tag}_user");
    let (status, body) = create_user_via_api(&app, &token, &username, &renamed).await;
    assert_eq!(status, StatusCode::OK, "建用户失败: {body}");
    let user_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("建用户响应里没有 id: {body}"));
    let summary = wait_for_audit_result("POST", "/api/admin/users", &username).await;
    assert!(
        summary.contains(&renamed),
        "建用户审计必须记下授予了哪些角色: {summary}"
    );

    // ── 11. 改用户角色 → 审计记下追加了哪个角色 ────────────────
    let user_update_path = format!("/api/admin/users/{user_id}");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &user_update_path,
            Some(&token),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "roles": [renamed.clone(), "user"],
                "is_active": true,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改用户失败: {body}");
    wait_for_audit_result("PUT", &user_update_path, "追加角色 user").await;

    // ── 12. 切状态 → 前后状态都要在 ───────────────────────────
    let status_path = format!("/api/admin/users/{user_id}/status");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &status_path,
            Some(&token),
            Some(json!({ "is_active": false })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "停用用户失败: {body}");
    let summary = wait_for_audit_result("PUT", &status_path, "状态由启用改为停用").await;
    assert!(
        summary.contains(&username),
        "停用审计必须点名是哪个账号: {summary}"
    );
    // 复位，免得下一条"追加角色"断言被自身的会话吊销影响
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &status_path,
            Some(&token),
            Some(json!({ "is_active": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "启用用户失败: {body}");

    // ── 13. 追加角色 → 审计记下授给谁、授了什么 ────────────────
    // 用一个新角色而不是 `admin`：给用户追加 admin 会凭空多出一名管理员，
    // 后面"批量删除不能删光管理员"那条保护会被本用例自己搅乱
    let role_b = format!("{tag}_role_b");
    let role_b_id = create_role_via_api(&app, &token, &role_b).await;
    let append_path = format!("/api/admin/users/{user_id}/roles");
    let (status, body) = send(
        &app,
        request(
            "POST",
            &append_path,
            Some(&token),
            Some(json!({ "role_name": role_b })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "追加角色失败: {body}");
    let summary = wait_for_audit_result("POST", &append_path, "追加角色").await;
    assert!(
        summary.contains(&username),
        "追加角色审计必须点名是哪个用户: {summary}"
    );
    assert!(
        summary.contains(&role_b),
        "追加角色审计必须点名授了哪个角色: {summary}"
    );

    // ── 14. 重置口令 → 记下重置了谁，且**新口令不入库** ─────────
    // 全库风险最高的写操作：拿到新口令即等于登录成该账号
    let secret = format!("Pw{}", uuid::Uuid::new_v4().simple());
    let reset_path = format!("/api/admin/users/{user_id}/reset-password");
    let (status, body) = send(
        &app,
        request(
            "POST",
            &reset_path,
            Some(&token),
            Some(json!({ "password": secret })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重置口令失败: {body}");
    wait_for_audit_result("POST", &reset_path, &username).await;
    assert_eq!(
        audit_rows_containing(&secret).await,
        0,
        "新口令「{secret}」被写进了审计表——长期表里的明文口令等于永久泄露"
    );

    // ── 15. 自助改密 → 记下改了谁，且新旧口令都不入库 ─────────
    // 用一次性账号而不是 admin：改了 admin 的口令会让后续所有用例无法登录
    let self_name = format!("{tag}_self");
    let self_old = "user1234";
    let (status, body) = create_user_via_api(&app, &token, &self_name, "user").await;
    assert_eq!(status, StatusCode::OK, "建自助改密账号失败: {body}");
    let self_token = activated_token(&app, &self_name, self_old).await;
    let self_new = format!("Pw{}", uuid::Uuid::new_v4().simple());
    let (status, body) = send(
        &app,
        request(
            "PUT",
            "/api/auth/password",
            Some(&self_token),
            Some(json!({ "old_password": self_old, "new_password": self_new })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自助改密失败: {body}");
    wait_for_audit_result("PUT", "/api/auth/password", &self_name).await;
    assert_eq!(
        audit_rows_containing(&self_new).await,
        0,
        "新口令被写进了审计表"
    );

    // ── 16. 登出 → 摘要说明只注销当前会话 ─────────────────────
    // **必须重新登录**：改密刚刚吊销了该账号的全部会话，
    // 拿改密前那个令牌打登出只会得到 401——不是登出坏了，是令牌已经作废
    let self_token = login_token(&app, &self_name, &self_new).await;
    let (status, body) = send(
        &app,
        request("POST", "/api/auth/logout", Some(&self_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "登出失败: {body}");
    wait_for_audit_result("POST", "/api/auth/logout", &self_name).await;

    // ── 17. 批量删除 → 逐个记名字，而不是只记"删了 1 个" ───────
    let victim = format!("{tag}_victim");
    let (status, body) = create_user_via_api(&app, &token, &victim, "user").await;
    assert_eq!(status, StatusCode::OK, "建待删账号失败: {body}");
    let victim_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("建号响应里没有 id: {body}"));
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/users/batch-delete",
            Some(&token),
            Some(json!({ "ids": [victim_id] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "批量删除失败: {body}");
    wait_for_audit_result("POST", "/api/admin/users/batch-delete", &victim).await;
    assert!(!user_exists(&victim).await, "批量删除应已生效");

    // ── 18. 删用户 → 名字与原角色都要留存在审计里 ───────────────
    let (status, body) = send(
        &app,
        request("DELETE", &user_update_path, Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "删除用户失败: {body}");
    wait_for_audit_result("DELETE", &user_update_path, &username).await;
    assert!(!user_exists(&username).await, "删除用户应已生效");

    // ── 19. 字典：类型增改删 ──────────────────────────────────
    let dict_code = format!("{tag}_dict");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/dict/types",
            Some(&token),
            Some(json!({ "code": dict_code, "name": "审计探针字典" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新建字典类型失败: {body}");
    let dict_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("新建字典响应里没有 id: {body}"));
    let dict_path = format!("/api/admin/dict/types/{dict_id}");
    wait_for_audit_result("POST", "/api/admin/dict/types", &dict_code).await;

    let dict_code2 = format!("{tag}_dict_v2");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &dict_path,
            Some(&token),
            Some(json!({ "code": dict_code2, "name": "改名后的字典" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改字典类型失败: {body}");
    let summary = wait_for_audit_result("PUT", &dict_path, &dict_code2).await;
    assert!(
        summary.contains(&dict_code),
        "改 code 必须保留旧 code，否则按 code 取缓存的前端会静默取到另一份数据: {summary}"
    );

    // ── 20. 字典项增改删 ─────────────────────────────────────
    let label_a = format!("{tag}_项A");
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/dict/items",
            Some(&token),
            Some(json!({
                "dict_type_id": dict_id,
                "label": label_a,
                "value": "a",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "新建字典项失败: {body}");
    let item_id = body["data"]["id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("新建字典项响应里没有 id: {body}"));
    let item_path = format!("/api/admin/dict/items/{item_id}");
    wait_for_audit_result("POST", "/api/admin/dict/items", &label_a).await;

    let label_b = format!("{tag}_项B");
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &item_path,
            Some(&token),
            Some(json!({
                "dict_type_id": dict_id,
                "label": label_b,
                "value": "b",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改字典项失败: {body}");
    let summary = wait_for_audit_result("PUT", &item_path, &label_b).await;
    assert!(
        summary.contains(&label_a),
        "改字典项必须保留旧值: {summary}"
    );

    let (status, body) = send(&app, request("DELETE", &item_path, Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK, "删除字典项失败: {body}");
    wait_for_audit_result("DELETE", &item_path, &label_b).await;

    // ── 21. 删字典类型 → 名字必须留存（删除是级联的） ──────────
    let (status, body) = send(&app, request("DELETE", &dict_path, Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK, "删除字典类型失败: {body}");
    wait_for_audit_result("DELETE", &dict_path, &dict_code2).await;

    // ── 22. 刷新缓存与重置指标 ────────────────────────────────
    let (status, body) = send(
        &app,
        request("POST", "/api/admin/dict/refresh", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "刷新字典缓存失败: {body}");
    // v0.16.0 起这条摘要不再写"刷新字典缓存"这种无信息量的话，
    // 而是报出实际清掉/回填的数量。所以这里除了能查到摘要，
    // 还要确认摘要里**带得上数字**——只有动词没有数值的摘要，
    // 事后照样回答不了"当时到底清了没有"。
    let refresh_audit = wait_for_audit_result("POST", "/api/admin/dict/refresh", "字典缓存").await;
    assert!(
        refresh_audit.chars().any(|c| c.is_ascii_digit()),
        "刷新缓存的审计摘要应报出实际数量，实际：{refresh_audit}"
    );

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
    // 指标清零会抹掉"此前谁在高频调用"的证据，这条本身必须可审计
    wait_for_audit_result(
        "POST",
        "/api/admin/monitor/metrics/reset",
        "重置全部接口指标",
    )
    .await;

    // ── 23. 最后删角色 → **名字在行消失后仍可追溯** ────────────
    let (status, body) = delete_role(&app, &token, role_id).await;
    assert_eq!(status, StatusCode::OK, "删除角色失败: {body}");
    let summary =
        wait_for_audit_result("DELETE", &format!("/api/admin/roles/{role_id}"), &renamed).await;
    assert!(
        !role_still_exists(role_id).await,
        "角色应已删除（否则这条审计说明的是一次没发生的删除）"
    );
    assert!(!summary.is_empty(), "角色删除审计必须是可读摘要而不是空值");

    // 收尾：把第二个角色也删掉，避免留在共享测试库里
    let _ = delete_role(&app, &token, role_b_id).await;
}

/// 失败的写操作**不得**留下摘要
///
/// 摘要的含义是"这次真的改了"。被拒绝的请求什么都没发生，
/// 却记下"已授予/已删除"就是谎报——审计一旦开始说谎，
/// 比没有审计更危险：它会让人**不再去看**其他证据。
///
/// **这条用例承重的是"handler 的 push 时机"，不是 2xx 门禁**：
/// 实测把中间件里的 `is_success` 判断去掉，本用例依然全绿，
/// 因为现有 handler 全都在副作用成功之后才 push，门禁根本用不上。
/// 门禁本身由 `middleware::audit_log::summary_for` 的单测钉住。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_rejected_write_leaves_no_change_summary() {
    let app = app().await;
    let token = admin_token(&app).await;
    let tag = unique("rejected");

    // 内置角色不可删除：这条路径在守卫处就被挡下，不会碰数据库
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/roles/{}", admin_role_id(&pool().await).await),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "删内置角色应被拒绝: {body}"
    );

    // 同名建角色：唯一约束冲突，同样什么都没发生
    let name = format!("{tag}_dup");
    let (status, _) = send(
        &app,
        request(
            "POST",
            "/api/admin/roles",
            Some(&token),
            Some(json!({ "name": name, "description": "首次" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/roles",
            Some(&token),
            Some(json!({ "name": name, "description": "重名" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "重名应冲突: {body}");

    // 两条被拒请求都写了一条审计记录（记录"有人试过"），
    // 但摘要必须是空的——不能让人以为角色被建了两次
    for (method, path) in [
        ("POST", "/api/admin/roles".to_string()),
        (
            "DELETE",
            format!("/api/admin/roles/{}", admin_role_id(&pool().await).await),
        ),
    ] {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT result FROM audit_logs \
             WHERE method = $1 AND path = $2 AND status_code >= 400 \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(method)
        .bind(&path)
        .fetch_optional(&pool().await)
        .await
        .expect("查询审计失败");
        let (result,) = row.expect("被拒的写请求也必须留审计（否则答不出「谁试过」）");
        assert!(
            result.as_deref().map(str::trim).unwrap_or("").is_empty(),
            "{method} {path} 被拒绝了却留下了变更摘要，审计在谎报: {result:?}"
        );
    }

    // 收尾：把首次建出来的角色删掉
    let id = role_id_by_name(&name).await;
    let _ = delete_role(&app, &token, id).await;
}

/// 取内置 `admin` 角色的 ID
async fn admin_role_id(pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM roles WHERE name = 'admin'")
        .fetch_one(pool)
        .await
        .expect("内置 admin 角色应当存在")
}

/// 写入口清单自检：OpenAPI 里新增的写操作必须被本节的用例覆盖
///
/// 上一条用例靠**手写**的调用序列驱动，因此新增端点不会自动被测到——
/// 那正是"测试自己也在说谎"的典型形态。这里从文档派生写操作集合，
/// 与本节真正执行过的端点清单比对，新增一个写端点却没纳入审计断言时会当场变红。
#[test]
fn every_documented_write_operation_is_covered_by_the_audit_test() {
    /// 本节用例实际执行过、并断言了摘要内容的写端点
    const COVERED: &[&str] = &[
        "POST /api/admin/roles",
        "PUT /api/admin/roles/{id}",
        "DELETE /api/admin/roles/{id}",
        "PUT /api/admin/roles/{role_id}/menus",
        "POST /api/admin/users/{user_id}/roles",
        "POST /api/admin/menus",
        "PUT /api/admin/menus/{id}",
        "DELETE /api/admin/menus/{id}",
        "POST /api/admin/menus/{id}/restore-permission",
        "POST /api/admin/users",
        "PUT /api/admin/users/{id}",
        "DELETE /api/admin/users/{id}",
        "POST /api/admin/users/batch-delete",
        "PUT /api/admin/users/{id}/status",
        "POST /api/admin/users/{id}/reset-password",
        "POST /api/admin/dict/types",
        "PUT /api/admin/dict/types/{id}",
        "DELETE /api/admin/dict/types/{id}",
        "POST /api/admin/dict/items",
        "PUT /api/admin/dict/items/{id}",
        "DELETE /api/admin/dict/items/{id}",
        "POST /api/admin/dict/refresh",
        "POST /api/admin/monitor/metrics/reset",
        "PUT /api/auth/password",
        "POST /api/auth/logout",
    ];

    /// 明确豁免的写端点：**每一条都要写出理由**，否则豁免就变成了漏测的挡箭牌
    const EXEMPT: &[(&str, &str)] = &[
        (
            "POST /api/auth/login",
            "公开路由，不在审计中间件内；由 AuthService 以语义 action \
             (AUTH_LOGIN_SUCCESS/FAILURE) 同步写审计，且 result 已记失败原因",
        ),
        (
            "POST /api/auth/register",
            "同上：AuthService 以 AUTH_REGISTER 同步写审计",
        ),
        (
            "POST /api/admin/validate",
            "纯入参校验演示，不改任何状态；给它编一条「变更摘要」才是谎报",
        ),
    ];

    let documented = openapi_operations()
        .into_iter()
        .filter(|(method, _, _)| matches!(method.as_str(), "POST" | "PUT" | "DELETE"))
        .map(|(method, path, _)| format!("{method} {path}"))
        .collect::<Vec<_>>();

    let mut unaccounted = Vec::new();
    for endpoint in &documented {
        let covered = COVERED.contains(&endpoint.as_str());
        let exempt = EXEMPT.iter().any(|(e, _)| e == endpoint);
        if !covered && !exempt {
            unaccounted.push(endpoint.clone());
        }
    }

    assert!(
        unaccounted.is_empty(),
        "以下写端点既没有被审计断言覆盖，也没有写明豁免理由：\n  {}",
        unaccounted.join("\n  ")
    );

    // 清单本身也要验：探针表整体失效时上面的循环空转，断言会全绿
    assert!(
        COVERED.len() >= 25,
        "审计断言覆盖的写端点只有 {} 条，覆盖清单可能已失效",
        COVERED.len()
    );
    for (endpoint, reason) in EXEMPT {
        assert!(
            !reason.trim().is_empty(),
            "写端点 {endpoint} 被豁免却没有给出理由"
        );
        assert!(
            documented.iter().any(|d| d == endpoint),
            "豁免清单里的 {endpoint} 已不在文档里，请删掉这条豁免"
        );
    }
    for endpoint in COVERED {
        assert!(
            documented.iter().any(|d| d == endpoint),
            "覆盖清单里的 {endpoint} 已不在文档里，请删掉这条覆盖"
        );
    }
}

// ============================================================
// v0.19.0：只测写端点，读端点烂七版没人知道
// ============================================================

/// 探针指向的端点
struct EndpointProbe {
    endpoint: String,
    method: String,
    /// 由文档派生出的具体路径（参数已填好）
    concrete: String,
    /// 该操作是否有请求体：有则发 `{}`，停在字段校验层
    has_body: bool,
    /// 是否是读端点（读端点额外要求 2xx，见下）
    is_read: bool,
}

/// **每一个**文档化端点都必须能被真实调用，且绝不 5xx
///
/// 回归的是 v0.19.0 的起点：`GET /api/admin/export/users` 从 v0.11.0 起
/// **每个调用都 500**（裸 SQL 漏了 `must_change_password`），却烂了七版。
/// 根因不是这处 SQL 写错，而是**守卫生效范围只覆盖写端点**——
/// `every_documented_write_operation_is_covered_by_the_audit_test` 从 OpenAPI
/// 派生的是 `POST|PUT|DELETE`，一个 GET 端点从不在它的视野里。
///
/// 所以这里把同一套"从文档派生"的手法扩到全部方法：
/// - **读端点（GET）必须 2xx**：它没有必填请求体，带合法令牌就该跑通。
///   返回 4xx 同样是缺陷——路由接到了却拒绝一个本该合法的请求。
/// - **写端点只要不是 5xx**：发 `{}` 让它停在校验层（400），
///   这样既走到了处理函数入口，又不改动任何真实数据。
///   `DELETE /api/admin/users/{VALID_UUID}` 之类打到不存在的行上是 404，不误删。
///
/// 唯一的副作用是 `POST /api/auth/logout` 会让令牌失效，探针跑完要重新登录；
/// `metrics/reset` 会清空指标，但每个读指标的用例都自己先重置，不受影响。
///
/// 为什么不写成"断言 50 处都调过了"：那种清单是**自证**——
/// 新增端点忘了登记，测试照样全绿。这里从 OpenAPI 派生，
/// 新端点一落地就自动进探针表。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn every_documented_endpoint_is_reachable_without_a_server_error() {
    let app = app().await;
    let mut token = admin_token(&app).await;

    // 探针要"因为到达处理函数而通过"，所以必须带合法令牌：
    // 无令牌时鉴权中间件先回 401，那是 JSON 信封，会让断言假绿
    let mut probes: Vec<EndpointProbe> = Vec::new();
    let mut read_count = 0usize;
    let mut write_count = 0usize;

    for (method, path, op) in openapi_operations() {
        let concrete = concrete_path(&path, &op);
        let is_read = method == "GET";
        if is_read {
            read_count += 1;
        } else {
            write_count += 1;
        }
        probes.push(EndpointProbe {
            endpoint: format!("{method} {path}"),
            method,
            concrete,
            // 带请求体的端点发 `{}`：能过内容协商与解析（证明提取器接好了），
            // 但会停在字段校验，不会真的建/改任何数据
            has_body: !op["requestBody"].is_null(),
            is_read,
        });
    }

    // 探针表自身也要验：文档结构一变导致一条都没派生出来时，
    // 上面的循环会空转、断言全绿——那正是本用例最怕的"因为没测到而通过"
    assert!(
        probes.len() >= 50,
        "从文档派生出的端点探针只有 {} 条，探针表可能已失效",
        probes.len()
    );
    assert!(
        read_count >= 22,
        "读端点探针只有 {read_count} 条，全局是否退化成只测写端点",
    );
    assert!(
        write_count >= 28,
        "写端点探针只有 {write_count} 条，探针表可能已失效",
    );

    let mut violations = Vec::new();
    for probe in probes {
        let EndpointProbe {
            endpoint,
            method,
            concrete,
            has_body,
            is_read,
        } = probe;
        // 请求必须在**此刻**用当前令牌构造，不能提前批量建好：
        // `POST /api/auth/logout` 会让令牌失效，而按 `(method, path)` 排序后
        // 它后面还跟着 `POST /api/auth/register` 与若干 PUT。
        // 提前构造会让这些探针带着已失效的令牌去跑，整串 401 假红。
        let body = has_body.then(|| json!({}));
        let (status, body) = send(&app, request(&method, &concrete, Some(&token), body)).await;

        if status.is_server_error() {
            violations.push(format!(
                "{endpoint} 返回了 {status}：{body}\n    · 处理函数内部出错，\
                 这正是 export/users 烂了七版的形态"
            ));
        } else if is_read && !status.is_success() {
            violations.push(format!(
                "{endpoint} 返回了 {status}：{body}\n    · 读端点无必填请求体，\
                 带合法令牌理应 2xx"
            ));
        }

        // logout 让当前令牌失效，下一个探针必须换新令牌
        if endpoint == "POST /api/auth/logout" {
            token = admin_token(&app).await;
        }
    }

    assert!(
        violations.is_empty(),
        "以下文档化端点被真实调用时出了问题（共探测 {} 个端点）：\n  {}",
        read_count + write_count,
        violations.join("\n  ")
    );
}

// ============================================================
// v0.14.0：审计会过期，但没人被告知
// ============================================================

/// 保留策略接口必须报告**当前部署的真实配置**，而不是一个写死的默认值
///
/// 回归的是 v0.14.0 的起点：`AUDIT_LOG_RETENTION_DAYS`（默认 90）会
/// 无条件删除过期审计行，而这件事此前只有进程 stdout 的一行
/// `tracing::info!`。界面查不到、接口查不到、README 也没写——
/// 于是"日志从某天起就查不到了"与"那天什么都没发生过"在管理员眼里
/// 完全一样，这个歧义本身就是审计的失效。
#[tokio::test]
#[ignore]
async fn the_retention_endpoint_reports_the_deployed_policy() {
    let app = app().await;
    let token = admin_token(&app).await;

    let (status, body) = send(
        &app,
        request("GET", "/api/admin/audit-logs/retention", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let d = &body["data"];
    // enabled 与 retention_days 必须自洽：不能一边说启用一边报 0 天
    let days = d["retention_days"]
        .as_i64()
        .expect("retention_days 必须是数字");
    assert_eq!(
        d["enabled"].as_bool(),
        Some(days > 0),
        "enabled 与 retention_days 互相矛盾：enabled={} days={days}",
        d["enabled"]
    );
    assert!(days > 0, "测试配置默认 90 天保留，不该为 0");
    assert!(
        d["oldest_log_at"].is_string(),
        "库里有日志，最老时刻应为字符串"
    );
    assert_eq!(
        d["cleanup_interval_seconds"].as_u64(),
        Some(3600),
        "清理间隔应当来自配置而不是写死"
    );
}

/// 一轮清理必须**自己留痕**，否则"日志为什么少了"只能在服务器日志里找
///
/// 判据是数据侧：清理跑完后 `audit_log_purges` 里真的有那一行，
/// 且接口能把 `cutoff_at` 与 `deleted_rows` 报出来。
/// 只断言"日志确实被删了"是不够的——删对了但不记，
/// 与本版之前的行为完全一样（那正是缺陷本身）。
#[tokio::test]
#[ignore]
async fn a_purge_is_recorded_so_the_deletion_can_be_found() {
    ensure_schema().await;
    let repo = AuditLogRepository::new(pool().await);
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(30);

    // 记录清理前的 purge 数量，便于结束时判定"确实是新写的这一行"
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log_purges")
        .fetch_one(&pool().await)
        .await
        .unwrap();

    let mut expired = Vec::new();
    for _ in 0..4 {
        expired.push(insert_audit_log(now - chrono::Duration::days(40)).await);
    }
    let kept = insert_audit_log(now - chrono::Duration::days(5)).await;

    let outcome = repo.delete_older_than(cutoff, 10, 5).await.unwrap();
    assert_eq!(outcome.deleted, 4, "应删掉 4 条过期日志");
    repo.record_purge(cutoff, &outcome, 7).await.unwrap();

    for id in &expired {
        assert!(!audit_log_exists(*id).await, "过期日志 {id} 应已被删除");
    }
    assert!(audit_log_exists(kept).await, "保留期内的日志不该被删");

    // 留痕本身可查：接口要能报出刚发生的那一轮
    let latest = repo
        .latest_purge()
        .await
        .unwrap()
        .expect("清理后应有 purge 记录");
    assert_eq!(latest.deleted_rows, 4);
    assert!(
        !latest.hit_batch_limit,
        "4 < 批大小 10，属清干净，不该报撞上限"
    );
    // cutoff 必须与实际删除用的是同一个值——否则界面显示的"从哪天起没了"是假的
    let trimmed: chrono::DateTime<chrono::Utc> = cutoff;
    assert_eq!(
        latest.cutoff_at.timestamp(),
        trimmed.timestamp(),
        "留痕的 cutoff 必须与删除用的是同一个时刻"
    );

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log_purges")
        .fetch_one(&pool().await)
        .await
        .unwrap();
    assert_eq!(after, before + 1, "应恰好新增一行清理记录");

    sqlx::query("DELETE FROM audit_logs WHERE id = $1")
        .bind(kept)
        .execute(&pool().await)
        .await
        .unwrap();
}

/// 撞上单轮批数上限时必须**如实报告**，不能让人以为已经清干净
///
/// 这是本版最容易说谎的一处：删了 20 条、还剩 5 条过期数据时，
/// 若只报"删了 20 条"，管理员会认为"清理已经完成"，
/// 而下一轮之前那 5 条其实一直躺在库里。
#[tokio::test]
#[ignore]
async fn a_purge_that_stops_at_the_batch_limit_says_so() {
    ensure_schema().await;
    let repo = AuditLogRepository::new(pool().await);
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(1);

    let mut ids = Vec::new();
    for _ in 0..12 {
        ids.push(insert_audit_log(now - chrono::Duration::days(5)).await);
    }

    // 批大小 10、上限 1 批 → 删满 10 条且预算用尽
    let outcome = repo.delete_older_than(cutoff, 10, 1).await.unwrap();
    assert_eq!(outcome.deleted, 10);
    assert!(
        outcome.hit_batch_limit,
        "删满一批且预算耗尽，必须报撞上限——否则 2 条残留会被当成已清干净"
    );
    repo.record_purge(cutoff, &outcome, 5).await.unwrap();

    let latest = repo.latest_purge().await.unwrap().unwrap();
    assert!(latest.hit_batch_limit, "接口层也必须把这个事实透出去");

    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_one(&pool().await)
        .await
        .unwrap();
    assert_eq!(left, 2, "确实还有 2 条过期数据留在库里");

    sqlx::query("DELETE FROM audit_logs WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&pool().await)
        .await
        .unwrap();
}

/// 保留策略接口不得越权，且要真放行持码者
///
/// 复用 `system:log:list` 而不是新增权限码，因此必须验证
/// "有码能看、没码看不到"两侧都成立。只测拒绝侧的话，
/// 一个"把端点整个禁掉"的实现也能全绿。
#[tokio::test]
#[ignore]
async fn the_retention_endpoint_obeys_the_log_permission() {
    use axum_api::model::permission;

    let app = app().await;
    let admin = admin_token(&app).await;

    // 放行侧：只持 system:log:list 的操作员能看，且真的拿到数据
    let (tok, role_id, uid) =
        operator_with_codes(&app, &admin, "retention_ok", &[permission::LOG_LIST]).await;
    let (status, body) = send(
        &app,
        request("GET", "/api/admin/audit-logs/retention", Some(&tok), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "持 log:list 应放行，body={body}");
    assert!(
        body["data"]["retention_days"].is_number(),
        "放行侧要真的拿到数据：200 但 body 里没有策略也算坏"
    );

    // 拒绝侧：匿名看不到
    let (status, _) = send(
        &app,
        request("GET", "/api/admin/audit-logs/retention", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "匿名不该看到保留策略");

    cleanup_operator(&app, &admin, uid, role_id).await;
}

// ──────────────────────────────────────────────
// 字典：管理页上的开关是否真的生效
// ──────────────────────────────────────────────
//
// v0.16.0 之前，字典模块 117 条集成测试里一条都没有——三个"控制"全部无效，
// 没有任何测试会发现：
//   1. status=disabled 的项照样出现在读取端点
//   2. is_default 可以同时有任意多个
//   3. 「刷新缓存」不删任何键，却返回"缓存刷新成功"
//
// 判据都落在**对外可观测的行为**上：读取端点返回什么、管理端还看不看得到、
// 端点报告的数字是多少。不去断言内部调用顺序。

/// 建一个字典类型，返回 (type_id, code)
async fn create_dict_type(app: &Router, token: &str) -> (String, String) {
    let code = unique("probe_dict");
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/dict/types",
            Some(token),
            Some(json!({ "code": code, "name": "探针字典" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建字典类型失败: {body}");
    (
        body["data"]["id"].as_str().unwrap().to_string(),
        body["data"]["code"].as_str().unwrap().to_string(),
    )
}

/// 往字典里加一项，返回 item_id
async fn create_dict_item(app: &Router, token: &str, type_id: &str, value: &str) -> String {
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/dict/items",
            Some(token),
            Some(json!({
                "dict_type_id": type_id,
                "label": value,
                "value": value,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建字典项失败: {body}");
    body["data"]["id"].as_str().unwrap().to_string()
}

/// 读取端点返回的 value 列表（业务页面真正拿到的数据）
async fn read_dict_values(app: &Router, token: &str, code: &str) -> Vec<String> {
    let (status, body) = send(
        app,
        request("GET", &format!("/api/dict/{code}/items"), Some(token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "读取字典失败: {body}");
    body["data"]
        .as_array()
        .expect("data 应为数组")
        .iter()
        .map(|i| i["value"].as_str().unwrap().to_string())
        .collect()
}

/// 管理端点看到的 (value, is_default) 列表
async fn list_dict_items_admin(app: &Router, token: &str, type_id: &str) -> Vec<(String, bool)> {
    let (status, body) = send(
        app,
        request(
            "GET",
            &format!("/api/admin/dict/items?dict_type_id={type_id}"),
            Some(token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "管理端读取字典项失败: {body}");
    body["data"]
        .as_array()
        .expect("data 应为数组")
        .iter()
        .map(|i| {
            (
                i["value"].as_str().unwrap().to_string(),
                i["is_default"].as_bool().unwrap(),
            )
        })
        .collect()
}

async fn set_dict_item_status(
    app: &Router,
    token: &str,
    item_id: &str,
    label: &str,
    value: &str,
    status: &str,
) -> (StatusCode, Value) {
    send(
        app,
        request(
            "PUT",
            &format!("/api/admin/dict/items/{item_id}"),
            Some(token),
            Some(json!({ "label": label, "value": value, "status": status })),
        ),
    )
    .await
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn disabled_dict_items_are_hidden_from_the_read_endpoint() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, code) = create_dict_type(&app, &admin).await;
    let kept = create_dict_item(&app, &admin, &type_id, "启用项").await;
    let hidden = create_dict_item(&app, &admin, &type_id, "禁用项").await;

    // 先确认两项都在——否则下面的断言会因为"本来就没数据"而空过
    assert_eq!(
        read_dict_values(&app, &admin, &code).await.len(),
        2,
        "前置条件：两项都应可读"
    );

    let (status, body) =
        set_dict_item_status(&app, &admin, &hidden, "禁用项", "禁用项", "disabled").await;
    assert_eq!(status, StatusCode::OK, "禁用字典项失败: {body}");

    // 核心判据：禁用项不再出现在读取端点
    let values = read_dict_values(&app, &admin, &code).await;
    assert_eq!(
        values,
        vec!["启用项".to_string()],
        "禁用项仍出现在读取端点，'禁用'开关没有生效"
    );
    assert!(
        !values.contains(&"禁用项".to_string()),
        "被禁用的项不应再被业务页面读到"
    );

    // 反向判据：管理页必须仍看得到它，否则管理员没法把它改回来
    let admin_view = list_dict_items_admin(&app, &admin, &type_id).await;
    assert_eq!(
        admin_view.len(),
        2,
        "管理页不该把禁用项藏起来——管理员需要看到并改回启用"
    );
    assert!(admin_view.iter().any(|(v, _)| v == "禁用项"));

    // 重新启用后立刻恢复，不该等缓存 TTL
    let (status, body) =
        set_dict_item_status(&app, &admin, &hidden, "禁用项", "禁用项", "enabled").await;
    assert_eq!(status, StatusCode::OK, "重新启用失败: {body}");
    let values = read_dict_values(&app, &admin, &code).await;
    assert_eq!(values.len(), 2, "重新启用后读取端点应恢复两项");

    let _ = kept;
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn disabled_dict_type_hides_every_item_it_owns() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, code) = create_dict_type(&app, &admin).await;
    create_dict_item(&app, &admin, &type_id, "甲").await;
    create_dict_item(&app, &admin, &type_id, "乙").await;
    assert_eq!(read_dict_values(&app, &admin, &code).await.len(), 2);

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/dict/types/{type_id}"),
            Some(&admin),
            Some(json!({ "code": code, "name": "探针字典", "status": "disabled" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "禁用字典类型失败: {body}");

    let values = read_dict_values(&app, &admin, &code).await;
    assert!(
        values.is_empty(),
        "类型已禁用，读取端点仍返回 {values:?}——'禁用'对类型层不生效"
    );

    // 重新启用后必须立刻恢复：禁用类型的空结果**不得**被缓存住，
    // 否则管理员刚点启用却要等 1 小时 TTL
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/dict/types/{type_id}"),
            Some(&admin),
            Some(json!({ "code": code, "name": "探针字典", "status": "enabled" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "重新启用字典类型失败: {body}");
    assert_eq!(
        read_dict_values(&app, &admin, &code).await.len(),
        2,
        "重新启用后应立刻恢复，不该等缓存过期"
    );
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn setting_a_new_default_item_clears_the_previous_one() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, _code) = create_dict_type(&app, &admin).await;
    let first = create_dict_item(&app, &admin, &type_id, "第一项").await;
    let second = create_dict_item(&app, &admin, &type_id, "第二项").await;

    for item_id in [&first, &second] {
        let label = if *item_id == first {
            "第一项"
        } else {
            "第二项"
        };
        let (status, body) = send(
            &app,
            request(
                "PUT",
                &format!("/api/admin/dict/items/{item_id}"),
                Some(&admin),
                Some(json!({ "label": label, "value": label, "is_default": true })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "设为默认失败: {body}");
    }

    let items = list_dict_items_admin(&app, &admin, &type_id).await;
    let defaults: Vec<&String> = items.iter().filter(|(_, d)| *d).map(|(v, _)| v).collect();
    assert_eq!(
        defaults.len(),
        1,
        "同一字典出现了 {defaults:?} 多个默认项——'默认'失去了唯一性"
    );
    assert_eq!(defaults[0], "第二项", "后设置的应成为唯一默认项");
}

/// **新建**路径的同一保证
///
/// 上一条只覆盖了 PUT（改）。注入"新建默认项时不再取消旧默认项"时全绿——
/// 因为 POST 那条路径根本没有用例。两条路径是两份独立代码，
/// 只测一条就等于把另一半放在"没人看过"的状态。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn creating_a_default_item_clears_the_previous_one() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, _code) = create_dict_type(&app, &admin).await;

    for value in ["甲", "乙"] {
        let (status, body) = send(
            &app,
            request(
                "POST",
                "/api/admin/dict/items",
                Some(&admin),
                Some(json!({
                    "dict_type_id": type_id,
                    "label": value,
                    "value": value,
                    "is_default": true,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "新建默认项失败: {body}");
    }

    let items = list_dict_items_admin(&app, &admin, &type_id).await;
    let defaults: Vec<&String> = items.iter().filter(|(_, d)| *d).map(|(v, _)| v).collect();
    assert_eq!(
        defaults.len(),
        1,
        "连续新建两个默认项后剩下 {defaults:?}——新建路径没有取消旧默认项"
    );
    assert_eq!(defaults[0], "乙", "后建的应成为唯一默认项");
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_disabled_dict_item_cannot_be_made_default() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, _code) = create_dict_type(&app, &admin).await;
    let item = create_dict_item(&app, &admin, &type_id, "待禁用项").await;

    let (status, _) =
        set_dict_item_status(&app, &admin, &item, "待禁用项", "待禁用项", "disabled").await;
    assert_eq!(status, StatusCode::OK);

    // 显式要求把禁用项设为默认 → 必须说清楚，而不是静默改写输入
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/dict/items/{item}"),
            Some(&admin),
            Some(json!({
                "label": "待禁用项",
                "value": "待禁用项",
                "status": "disabled",
                "is_default": true,
            })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "禁用项被设为默认项：读取端点会过滤掉它，'默认'将指向业务页面读不到的值"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("禁用"),
        "错误消息应说明为什么不行，实际：{body}"
    );

    let items = list_dict_items_admin(&app, &admin, &type_id).await;
    assert!(
        !items.iter().any(|(_, d)| *d),
        "被拒绝的写入不该留下默认标记：{items:?}"
    );
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn refresh_dict_cache_really_deletes_stale_keys() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, code) = create_dict_type(&app, &admin).await;
    create_dict_item(&app, &admin, &type_id, "真实项").await;

    // 先让缓存热起来，确认读取端确实走缓存（否则"刷新"无从谈起）
    assert_eq!(read_dict_values(&app, &admin, &code).await, vec!["真实项"]);

    // 直接往 Redis 塞一份陈旧数据，模拟写路径失效失败后管理员面对的局面
    let stale = json!([{
        "id": "11111111-1111-1111-1111-111111111111",
        "label": "陈旧项", "value": "STALE", "sort_order": 0,
        "status": "enabled", "is_default": false, "color": Value::Null,
    }])
    .to_string();
    let redis = RedisClient::new(&test_config(1_000).redis)
        .await
        .expect("连接 Redis 失败");
    redis
        .set_string(&format!("dict:{code}"), &stale, 3600)
        .await
        .expect("写入陈旧缓存失败");
    assert_eq!(
        read_dict_values(&app, &admin, &code).await,
        vec!["STALE"],
        "前置条件：陈旧缓存应生效"
    );

    let (status, body) = send(
        &app,
        request("POST", "/api/admin/dict/refresh", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "刷新缓存失败: {body}");

    // 核心判据：刷新之后读到的是数据库里的真实数据
    assert_eq!(
        read_dict_values(&app, &admin, &code).await,
        vec!["真实项"],
        "刷新缓存后仍在读陈旧数据——这个按钮什么都没做"
    );
}

#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn refresh_dict_cache_reports_the_number_of_keys_it_deleted() {
    let app = app().await;
    let admin = admin_token(&app).await;
    let (type_id, code) = create_dict_type(&app, &admin).await;
    create_dict_item(&app, &admin, &type_id, "甲").await;
    let (_t2, code2) = create_dict_type(&app, &admin).await;
    create_dict_item(&app, &admin, &_t2, "乙").await;

    // 两份字典都读一次，把缓存热起来
    read_dict_values(&app, &admin, &code).await;
    read_dict_values(&app, &admin, &code2).await;

    let (status, body) = send(
        &app,
        request("POST", "/api/admin/dict/refresh", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "刷新缓存失败: {body}");

    let cleared = body["data"]["cleared_keys"]
        .as_u64()
        .expect("缺少 cleared_keys");
    let reloaded = body["data"]["reloaded_types"]
        .as_u64()
        .expect("缺少 reloaded_types");
    assert!(
        cleared >= 2,
        "报告只清掉了 {cleared} 个键，而本用例至少造了 2 份热缓存"
    );
    assert!(
        reloaded >= 2,
        "报告只回填了 {reloaded} 个类型，本用例至少建了 2 个"
    );

    // 再点一次。此刻缓存里正好只有上一次回填写进去的那些键，
    // 所以"第二次清理数 == 第一次回填数"。这个等式比"报 0"更强：
    // 它证明报出来的数字是**真数出来的**，不是固定值也不是上一轮的回显。
    let (status, body) = send(
        &app,
        request("POST", "/api/admin/dict/refresh", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second = body["data"]["cleared_keys"]
        .as_u64()
        .expect("缺少 cleared_keys");
    assert_eq!(
        second, reloaded,
        "第二次清理的键数应等于第一次回填的类型数（缓存里正好只有那批键）"
    );
}

// ══════════════════════════════════════════════════════════════
// v0.17.0：菜单树成环 → 整棵子树静默消失 → 删除永久挂起 → 全站 500
//
// 这一组的每个用例都对应一条**实测复现过**的破坏链环节。
// 断言写在"外部可观测后果"上（HTTP 状态、树里还在不在、请求会不会返回），
// 而不是"某个私有函数返回了 Err"——后者只能证明代码跑到了，证明不了行为。
// ══════════════════════════════════════════════════════════════

/// 造一棵两层的临时菜单（根 → 子），返回 (根 id, 子 id)
async fn make_two_level_menu(app: &Router, tok: &str) -> (uuid::Uuid, uuid::Uuid) {
    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/menus",
            Some(tok),
            Some(json!({
                "name": unique("v017_dir"),
                "type": "directory",
                "sort_order": 97
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建根菜单失败: {body}");
    let root = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let (status, body) = send(
        app,
        request(
            "POST",
            "/api/admin/menus",
            Some(tok),
            Some(json!({
                "parent_id": root,
                "name": unique("v017_leaf"),
                "type": "menu",
                "path": format!("/{}", unique("v017p")),
                "sort_order": 1
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建子菜单失败: {body}");
    let leaf = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    (root, leaf)
}

/// 直查库里某个菜单当前的 `parent_id`（`None` 表示它是根）
///
/// 刻意查库而不是看响应体：v0.17.0 之前最恶劣的一个症状就是
/// **响应体原样回显了没生效的 `parent_id`**——只看响应会以为成功了。
async fn parent_in_db(id: uuid::Uuid) -> Option<uuid::Uuid> {
    sqlx::query_scalar("SELECT parent_id FROM menus WHERE id = $1")
        .bind(id)
        .fetch_one(&pool().await)
        .await
        .expect("查询菜单父级失败")
}

/// 菜单名出现在管理页菜单树里吗
async fn menu_visible_in_tree(app: &Router, tok: &str, name: &str) -> bool {
    let (status, body) = send(app, request("GET", "/api/admin/menus", Some(tok), None)).await;
    assert_eq!(status, StatusCode::OK, "读取菜单树失败: {body}");

    let mut found = false;
    fn walk(nodes: &Value, needle: &str, found: &mut bool) {
        for n in nodes.as_array().into_iter().flatten() {
            if n["name"].as_str() == Some(needle) {
                *found = true;
            }
            walk(&n["children"], needle, found);
        }
    }
    walk(&body["data"], name, &mut found);
    found
}

/// 菜单不能把自己设为自己的上级
///
/// 这是破坏链的**起点**：一次 `PUT` 返回 200 就能让一棵子树从界面上消失。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_menu_cannot_be_its_own_parent() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, _) = make_two_level_menu(&app, &token).await;

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{root}"),
            Some(&token),
            Some(json!({ "parent_id": root })),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "把自己设为上级必须被拒: {body}"
    );
    // 关键：库里必须真的没变。返回 400 但数据照样写进去，等于没修。
    assert_eq!(
        parent_in_db(root).await,
        None,
        "自引用被拒后，父级必须保持为根"
    );

    cleanup_temp_menu_dir(&app, &token, root).await;
}

/// 不能把菜单挪到它自己的子孙下面（成环）
///
/// 这一条才是真正会造成"整棵子树静默消失 + 删除永久挂起"的那个操作。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_menu_cannot_be_moved_under_its_own_descendant() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, leaf) = make_two_level_menu(&app, &token).await;
    let root_name = format!("{root}");

    // 先记下叶子在管理页的名字：成环之后它必须**仍在**树上
    let (status, body) = send(&app, request("GET", "/api/admin/menus", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    let leaf_name = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|n| {
            let mut acc = Vec::new();
            let mut stack = vec![n.clone()];
            while let Some(x) = stack.pop() {
                if x["id"].as_str() == Some(leaf.to_string().as_str()) {
                    acc.push(x["name"].as_str().unwrap_or_default().to_string());
                }
                if let Some(ch) = x["children"].as_array() {
                    stack.extend(ch.iter().cloned());
                }
            }
            acc
        })
        .next()
        .expect("临时子菜单应出现在菜单树里");
    assert!(!leaf_name.is_empty(), "子菜单名不应为空: {root_name}");

    // 把根挂到叶子下面 ⇒ 根→叶子→根
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{root}"),
            Some(&token),
            Some(json!({ "parent_id": leaf })),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "挂到自己的子孙下必须被拒: {body}"
    );
    assert_eq!(
        parent_in_db(root).await,
        None,
        "成环被拒后，根菜单必须仍然是根"
    );
    assert!(
        menu_visible_in_tree(&app, &token, &leaf_name).await,
        "成环被拒后，子菜单必须仍在菜单树里"
    );

    cleanup_temp_menu_dir(&app, &token, root).await;
}

/// 挂到不存在的上级上返回 400，而不是 500「服务器内部错误」
///
/// 这是**入参问题**：外键能挡住写入，但抛出来的 500 会让管理员看不懂，
/// 也会把错误监控污染成服务端故障。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn moving_a_menu_under_an_unknown_parent_is_a_bad_request() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, _) = make_two_level_menu(&app, &token).await;
    let ghost = uuid::Uuid::new_v4();

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{root}"),
            Some(&token),
            Some(json!({ "parent_id": ghost })),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "挂到不存在的上级应报 400: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("上级菜单不存在"),
        "错误消息要说清是上级菜单的问题: {body}"
    );
    assert_eq!(parent_in_db(root).await, None, "父级必须保持为根");

    cleanup_temp_menu_dir(&app, &token, root).await;
}

/// `parent_id: null` 真的把菜单摘成根（此前返回 200 却什么也没做）
///
/// 这是"管理员没有任何途径调整菜单层级"的直接成因：
/// `Option<Uuid>` 让「没传」与「传 null」不可区分，后者被当成"本次不改"。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_null_parent_id_actually_detaches_the_menu_to_the_top_level() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, leaf) = make_two_level_menu(&app, &token).await;
    assert_eq!(
        parent_in_db(leaf).await,
        Some(root),
        "前置条件：叶子应挂在根下"
    );

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "parent_id": null })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "摘成根应成功: {body}");
    // 响应体和数据库**都要**查：v0.17.0 之前响应体原样回显了旧父级，
    // 只看响应会漏掉这个缺陷。
    assert_eq!(
        body["data"]["parent_id"].as_str(),
        None,
        "响应体应回显 parent_id=null"
    );
    assert_eq!(parent_in_db(leaf).await, None, "库里的父级必须真的被清空");

    // 摘成根后它仍然在树上，且成了顶层节点
    let (status, body) = send(&app, request("GET", "/api/admin/menus", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    let top_level_ids: Vec<&str> = body["data"]
        .as_array()
        .expect("菜单树应是数组")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(
        top_level_ids.contains(&leaf.to_string().as_str()),
        "摘成根的菜单应出现在顶层"
    );

    cleanup_temp_menu_dir(&app, &token, root).await;
    cleanup_temp_menu_dir(&app, &token, leaf).await;
}

/// 没传 `parent_id` 时保持原样（区分"没传"与"传 null"的前提）
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn omitting_parent_id_leaves_it_unchanged() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, leaf) = make_two_level_menu(&app, &token).await;

    // 只改名字，不提父级
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "name": unique("v017_renamed") })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "改名应成功: {body}");
    assert_eq!(
        parent_in_db(leaf).await,
        Some(root),
        "没传 parent_id 时父级必须保持不变"
    );

    cleanup_temp_menu_dir(&app, &token, root).await;
}

/// 合法改父级确实生效（确认校验没有把正常操作一起拦掉）
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_legitimate_reparent_takes_effect() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root_a, leaf) = make_two_level_menu(&app, &token).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(json!({ "name": unique("v017_other"), "type": "directory", "sort_order": 96 })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建第二个根菜单失败: {body}");
    let root_b = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "parent_id": root_b })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "合法改父级应成功: {body}");
    assert_eq!(parent_in_db(leaf).await, Some(root_b), "父级应真的改掉");

    cleanup_temp_menu_dir(&app, &token, root_a).await;
    cleanup_temp_menu_dir(&app, &token, root_b).await;
}

/// 库里已经有环时，`DELETE` 不会再永久挂起
///
/// 这是本版最重的一条。v0.17.0 之前 `granted_codes_in_subtree` 用
/// `UNION ALL` 递归，环上永不收敛 → 请求不返回、连接不归还 →
/// 占满连接池后**与菜单无关的端点也全部 500**。
///
/// 环是**直连 SQL 造的**：API 已经被拦住，这里要证明的是第二道防线
/// （`UNION` 去重）在遇到历史脏数据 / 运维直连写入时仍能收住。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn deleting_a_menu_does_not_hang_when_the_tree_contains_a_cycle() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, leaf) = make_two_level_menu(&app, &token).await;

    // 绕过 API 直接造环：根→叶子→根
    sqlx::query("UPDATE menus SET parent_id = $2 WHERE id = $1")
        .bind(root)
        .bind(leaf)
        .execute(&pool().await)
        .await
        .expect("造环失败");

    // 超时兜底：万一又挂起，测试会**失败**而不是把整个套件拖死
    let deleted = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        send(
            &app,
            request(
                "DELETE",
                &format!("/api/admin/menus/{root}"),
                Some(&token),
                None,
            ),
        ),
    )
    .await;

    assert!(
        deleted.is_ok(),
        "树上存在环时，DELETE 仍然永久挂起——递归 CTE 没终止"
    );

    // 删除本身也要真的生效：级联会顺着 parent_id 把叶子一起带走
    assert!(
        !menu_still_exists(root).await && !menu_still_exists(leaf).await,
        "删除环上的根菜单后，环上其余节点应随级联一并消失"
    );

    // 删完必须能立刻响应下一个请求：挂起时连接不归还，这才是全站 500 的成因
    let (status, body) = send(
        &app,
        request("GET", "/api/admin/menus/diagnostics", Some(&token), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "删除请求返回后服务应立即可用: {body}"
    );

    // 级联已经把叶子一并带走了（leaf.parent_id = root），
    // 所以这里只需在"确实还留着"时再清一次——重复 DELETE 会拿到 404。
    if menu_still_exists(leaf).await {
        // 先剪环再清理：万一上面某条断言没过，残留也不会是个挂死清理助手的环
        flatten_menus_to_roots(leaf).await;
        cleanup_temp_menu_dir(&app, &token, leaf).await;
    }
}

/// 诊断口能报出成环的节点，并能用公开 API 把它救回来
///
/// 修复动作刻意复用 `PUT .../menus/:id` + `parent_id: null`，
/// 不新增第二条写路径——那样又多一处需要同样权限守卫的地方。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn diagnostics_report_menus_trapped_in_a_cycle_and_they_can_be_rescued() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root, leaf) = make_two_level_menu(&app, &token).await;

    // 干净起点：没有环
    let (status, body) = send(
        &app,
        request("GET", "/api/admin/menus/diagnostics", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "读取诊断失败: {body}");
    assert_eq!(
        body["data"].as_array().map(Vec::len),
        Some(0),
        "干净库里不应报出不可达节点: {body}"
    );

    let root_name = sqlx::query_scalar::<_, String>("SELECT name FROM menus WHERE id = $1")
        .bind(root)
        .fetch_one(&pool().await)
        .await
        .expect("查询菜单名失败");
    let leaf_name = sqlx::query_scalar::<_, String>("SELECT name FROM menus WHERE id = $1")
        .bind(leaf)
        .fetch_one(&pool().await)
        .await
        .expect("查询菜单名失败");

    // 造环（绕过 API 模拟历史脏数据）
    sqlx::query("UPDATE menus SET parent_id = $2 WHERE id = $1")
        .bind(root)
        .bind(leaf)
        .execute(&pool().await)
        .await
        .expect("造环失败");

    // 环上两个节点都从菜单树上消失了——这正是"管理员看不见、也就修不了"的由来
    assert!(
        !menu_visible_in_tree(&app, &token, &root_name).await,
        "成环后根菜单不应出现在菜单树里（这正是无法自救的原因）"
    );
    assert!(
        !menu_visible_in_tree(&app, &token, &leaf_name).await,
        "成环后叶子菜单也不应出现在菜单树里：整支被剪掉，不是只剪环上的父节点"
    );

    let (status, body) = send(
        &app,
        request("GET", "/api/admin/menus/diagnostics", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "读取诊断失败: {body}");
    let broken = body["data"].as_array().expect("诊断应返回数组").clone();
    let broken_ids: Vec<String> = broken
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    assert!(
        broken_ids.contains(&root.to_string()) && broken_ids.contains(&leaf.to_string()),
        "诊断应报出环上的两个节点，实际: {body}"
    );
    assert!(
        broken[0]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("成环"),
        "应说明是成环而不是别的: {body}"
    );

    // 自救：把根摘成根节点，环就破了
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{root}"),
            Some(&token),
            Some(json!({ "parent_id": null })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "摘成根应能救回来: {body}");

    // 破环后根重新出现在树上
    assert!(
        menu_visible_in_tree(&app, &token, &root_name).await,
        "破环后根菜单应重新出现在菜单树里"
    );

    // 把剩下的叶子也摘出来，诊断归零
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "parent_id": null })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "摘成根应成功: {body}");

    let (status, body) = send(
        &app,
        request("GET", "/api/admin/menus/diagnostics", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"].as_array().map(Vec::len),
        Some(0),
        "全部救回后诊断应为空: {body}"
    );

    flatten_menus_to_roots(root).await;
    flatten_menus_to_roots(leaf).await;
    cleanup_temp_menu_dir(&app, &token, root).await;
    cleanup_temp_menu_dir(&app, &token, leaf).await;
}

/// 造环的操作会留下审计痕迹，且说清是移动而非静默改结构
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn moving_a_menu_is_recorded_in_the_audit_log() {
    let app = app().await;
    let token = admin_token(&app).await;
    let (root_a, leaf) = make_two_level_menu(&app, &token).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/menus",
            Some(&token),
            Some(json!({ "name": unique("v017_dst"), "type": "directory", "sort_order": 95 })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let root_b = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();

    let dst_name: String = sqlx::query_scalar("SELECT name FROM menus WHERE id = $1")
        .bind(root_b)
        .fetch_one(&pool().await)
        .await
        .expect("查询目标目录名失败");

    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "parent_id": root_b })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "移动应成功: {body}");

    // 审计里必须有"移动"字样并点名新上级；此前结构性变更**完全不留痕**
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM audit_logs
             WHERE result LIKE '%移动菜单%' AND result LIKE $1
         )",
    )
    .bind(format!("%{dst_name}%"))
    .fetch_one(&pool().await)
    .await
    .expect("查询审计失败");
    assert!(found, "移动菜单应在审计里留痕并点名新上级");

    // 摘成根也要留痕
    let (status, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/admin/menus/{leaf}"),
            Some(&token),
            Some(json!({ "parent_id": null })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "摘成根应成功: {body}");
    let detached: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM audit_logs WHERE result LIKE '%摘成根菜单%')",
    )
    .fetch_one(&pool().await)
    .await
    .expect("查询审计失败");
    assert!(detached, "摘成根应在审计里留痕");

    // 叶子已被摘成根，不再随 root_a/root_b 级联——必须单独删，
    // 否则每次跑这个用例都在测试库里留一条孤儿菜单
    cleanup_temp_menu_dir(&app, &token, leaf).await;
    cleanup_temp_menu_dir(&app, &token, root_a).await;
    cleanup_temp_menu_dir(&app, &token, root_b).await;
}

// ══════════════════════════════════════════════════════════════════
// v0.18.0：口令策略不得挂到登录路径（HTTP 层约束）
// ══════════════════════════════════════════════════════════════════

/// 造一个"v0.11.0 之前就存在"的弱口令用户
///
/// 直接写 SQL 插入，哈希用 `axum_api::utils::password::hash_password`
/// ——**绕过策略校验**正是这里的目的：策略是"设置口令"时的一道闸，
/// 而闸门修好之后，闸门**之前**进来的存量用户必须还能从闸门走出去。
async fn drop_user(username: &str) {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(&pool().await)
        .await
        .expect("清理存量用户失败");
}

async fn insert_legacy_user(username: &str, password: &str) {
    let hash = axum_api::utils::password::hash_password(password).expect("哈希计算失败");
    sqlx::query(
        "INSERT INTO users (username, email, password_hash, is_active, must_change_password) \
         VALUES ($1, $2, $3, TRUE, FALSE)",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .bind(hash)
    .execute(&pool().await)
    .await
    .expect("插入存量用户失败");
}

/// 存量弱口令用户仍能经 API 登录
///
/// 把 `password_policy_is_not_applied_to_login_verification`（单元测试）
/// 升到 HTTP 层。单元测试只能证明"`validate_password` 会对这个口令报错"，
/// 证不了**登录链路真的不会去调它**——而后者才是会锁死人的那个性质。
///
/// 这条性质的失效方式极其隐蔽：把 `validate_password` 加进
/// `handle_login` 会让所有测试照样绿（测试用户口令全是合规的），
/// 直到真实用户集体登不上系统。所以要在接口层钉住。
///
/// 顺带钉住两件**必须一起成立**的事：弱口令能被拒于设置、却不被拒于登录。
/// 只测其中一头的话，两种错误实现都能通过。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_legacy_weak_password_user_can_still_log_in() {
    let app = app().await;
    let name = unique("legacy");

    // v0.10 时代的策略只有"至少 6 位"，所以 6 位纯小写是**当年合法**的口令
    let weak = "abcdef";

    // 前置事实：这个口令确实**不满足现行策略**——否则下面那句断言是空话
    assert!(
        axum_api::utils::validation::validate_password(weak).is_err(),
        "样例口令 {weak:?} 本应不满足现行策略；换策略后这条用例的前提要重挑"
    );

    insert_legacy_user(&name, weak).await;

    // 该走的路：能被登录接受，并拿到可用的令牌
    //
    // **先取结果、清理、最后断言**：断言一旦红就跳到 panic，
    // 写在后面的清理永远不会执行——那条断言本身正在测的场景
    // （登录被拒）恰恰最容易让清理被跳过，于是每次红一次漏一个账号。
    let (login_status, login_body) = login(&app, &name, weak).await;
    let usable = if login_status == StatusCode::OK {
        let tok = login_body["data"]["token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let (status, _) = send(
            &app,
            request("GET", "/api/auth/permissions", Some(&tok), None),
        )
        .await;
        status == StatusCode::OK
    } else {
        false
    };

    // 不该走的路：同一个口令**不能**被拿来设新号。两条一起断言，
    // 防止有人"为了让上面那条通过"而把策略从设置路径上一并删掉
    let (set_status, set_body) = send(
        &app,
        request(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "username": unique("weaknew"),
                "email": format!("{name}@probe.example.com"),
                "password": weak,
            })),
        ),
    )
    .await;

    drop_user(&name).await;

    assert!(usable, "弱口令存量用户登录后应能正常使用系统");
    assert_eq!(
        set_status,
        StatusCode::BAD_REQUEST,
        "弱口令在设置时仍应被拒（否则策略被误删）: {set_body}"
    );
}

/// 超长口令：策略在"设置"时拒，但存量用户仍能登录
///
/// 与上一条同类，但换了个方向卡住**长度上限**。
///
/// 登录页原来写死 `maxlength="128"`，可后端登录验的是 Argon2 哈希，
/// 不看明文长度。若某天把长度规则也搬进登录路径，
/// 持有超长口令的存量用户会在前端被直接挡在输入框那一层——
/// 连"密码错误"这句话都看不到，只会觉得"系统坏了"。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_legacy_overlong_password_user_can_still_log_in() {
    let app = app().await;
    let name = unique("longpw");

    // 200 字符，三类字符齐全——**只有长度**这一关不过。
    // 用合规字符类型是关键：这样"被拒"的归因只能是长度，不是复杂度
    let overlong = format!("Ab1{}", "x".repeat(197));
    assert_eq!(overlong.chars().count(), 200);
    assert!(
        axum_api::utils::validation::validate_password(&overlong).is_err(),
        "样例口令本应因超长被现行策略拒收"
    );

    insert_legacy_user(&name, &overlong).await;

    // 同上：先取结果再清理，避免失败路径漏数据
    let (login_status, login_body) = login(&app, &name, &overlong).await;
    let usable = if login_status == StatusCode::OK {
        let tok = login_body["data"]["token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let (status, _) = send(
            &app,
            request("GET", "/api/auth/permissions", Some(&tok), None),
        )
        .await;
        status == StatusCode::OK
    } else {
        false
    };

    drop_user(&name).await;
    assert!(usable, "超长口令存量用户登录后应能正常使用系统");
}

/// 长邮箱用户能用邮箱登录，且注册接口不设比数据库更严的上限
///
/// 钉住登录页 `max: 50` → `max: 255` 这次修改的依据：
/// `users.email` 是 `varchar(255)`，注册接口也接受长到这个上限的邮箱。
/// 前端若把上限写死成 50，持有长邮箱的合法用户会在**自己的登录页上
/// 敲不进自己的邮箱**，而后端从头到尾都认。
#[tokio::test]
#[ignore = "需要真实 Postgres + Redis"]
async fn a_long_email_can_be_used_to_log_in() {
    let app = app().await;
    let name = unique("longmail");
    // 73 字符：合法邮箱，明显超过旧前端规则的 50。
    //
    // **局部部分必须每次唯一**：`users.email` 上有唯一约束，
    // 而固定邮箱意味着"上一次失败后残留的账号"会让这一次直接撞唯一键。
    // 之前就踩过：e2e 挂死那次留下的账号用了同一个固定邮箱，
    // 导致这条用例在整轮里报了一个与被测性质无关的 `duplicate key`。
    // 前缀拼在 40 个 a 之后（而不是替代它）——否则总长会掉到 50 整，
    // 恰好等于旧前端的上限，这条用例就不再能证明任何事了。
    // 下面的 assert 把这个前提钉住：长度一变就红，而不是悄悄退化成空测试。
    let email = format!(
        "{}{}@bbbbbbbbbbbbbbbbbbbb.example.com",
        "a".repeat(40),
        name
    );
    assert!(
        email.chars().count() > 50,
        "样例邮箱必须长于旧前端的 50 上限，当前只有 {} 个字符，这条用例就白写了",
        email.chars().count()
    );

    insert_legacy_user(&name, "Abcdef12").await;
    sqlx::query("UPDATE users SET email = $2 WHERE username = $1")
        .bind(&name)
        .bind(&email)
        .execute(&pool().await)
        .await
        .expect("改邮箱失败");

    let (login_status, login_body) = login(&app, &email, "Abcdef12").await;
    let usable = if login_status == StatusCode::OK {
        let tok = login_body["data"]["token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let (status, _) = send(
            &app,
            request("GET", "/api/auth/permissions", Some(&tok), None),
        )
        .await;
        status == StatusCode::OK
    } else {
        false
    };

    drop_user(&name).await;
    assert!(usable, "长邮箱应能用于登录");
}

/// 剥掉 JS/TS 注释，只留下真正会执行的代码
///
/// 这条剥离是 `account_forms_use_the_shared_validator` 能成立的前提。
/// 那几个页面里都留着解释修复来由的注释，**里面就写着 `min: 6`**——
/// 因为"v0.18.0 之前这里是 `min: 6`"正是最该被保留的那句话。
/// 裸 `contains` 会把这段注释判成违规，于是要么测试永远红，
/// 要么有人去删掉有用的注释来讨好测试。两种结局都比缺陷本身更糟。
///
/// 状态机而非正则：正则分不清 `//` 是注释还是字符串里的一部分
/// （URL、`'https://…'` 这类），而误判方向恰好是**把真代码当注释删掉**——
/// 那会让守卫在真正有 `min: 6` 时依然绿。
fn strip_js_comments(source: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Code,
        LineComment,
        BlockComment,
        SingleQuote,
        DoubleQuote,
        Backtick,
    }
    let mut out = String::with_capacity(source.len());
    let mut state = State::Code;
    let mut chars = source.chars().peekable();

    while let Some(c) = chars.next() {
        match state {
            State::Code => match c {
                '/' if chars.peek() == Some(&'/') => {
                    chars.next();
                    state = State::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    state = State::BlockComment;
                }
                '\'' => {
                    state = State::SingleQuote;
                    out.push(c);
                }
                '"' => {
                    state = State::DoubleQuote;
                    out.push(c);
                }
                '`' => {
                    state = State::Backtick;
                    out.push(c);
                }
                _ => out.push(c),
            },
            State::LineComment => {
                if c == '\n' {
                    state = State::Code;
                    out.push(c);
                }
            }
            State::BlockComment => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = State::Code;
                }
            }
            State::SingleQuote | State::DoubleQuote | State::Backtick => {
                if c == '\\' {
                    // 转义：连下一个字符一起吞掉，否则 `'\''` 会提前闭合
                    out.push(c);
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else {
                    if (state == State::SingleQuote && c == '\'')
                        || (state == State::DoubleQuote && c == '"')
                        || (state == State::Backtick && c == '`')
                    {
                        state = State::Code;
                    }
                    out.push(c);
                }
            }
        }
    }
    out
}

/// 只取 `<script>` 块，并剥掉其中的注释
///
/// `<template>` 与 `<style>` 里不该出现校验规则，但它们也常带说明性文字。
/// 收窄到 script 既贴合意图，也少一类误报来源。
fn script_code_of(vue_source: &str) -> String {
    let start = vue_source
        .find("<script")
        .expect("vue 文件里找不到 <script> 块");
    let after = &vue_source[start..];
    let end = after.find("</script>").expect("<script> 块未闭合");
    strip_js_comments(&after[..end])
}

/// 前后端用户名校验**必须给出同一结论**
///
/// 与 `password_policy_agrees_with_the_frontend_copy` 同构，但换了一条轴。
///
/// 用户名规则在前端 `utils/accountRules.ts` 与后端 `validation.rs` 各存一份，
/// 而这次的漂移比口令那次更隐蔽：**两边都是"合法"的**，
/// 差别只在于前端放行了一批后端会拒的字符（或反过来），
/// 于是用户看到的是"前端全绿 → 提交 → 400 用户名只能包含…"。
///
/// 做法同样是从前端源码里解析样例表，用 Rust 跑同一批取值再比对结论。
#[test]
fn username_policy_agrees_with_the_frontend_rules() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("frontend/src/utils/accountRules.ts"))
        .expect("读取 frontend/src/utils/accountRules.ts 失败");

    // 同样只在数组体内解析：该文件顶部的文档注释里也有 `{ name: '...', ok: true }`
    let body_start = source
        .find("export const USERNAME_POLICY_CASES")
        .expect("前端未导出 USERNAME_POLICY_CASES");
    let body = &source[body_start..];
    let body_end = body
        .find("\n]")
        .expect("USERNAME_POLICY_CASES 数组未正常闭合");
    let body = &body[..body_end];

    let mut cases: Vec<(String, bool)> = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("{ name: '") {
        let after = &rest[at + "{ name: '".len()..];
        let Some(end_quote) = after.find('\'') else {
            break;
        };
        let name = after[..end_quote].to_string();
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
        cases.push((name, verdict.starts_with("true")));
        rest = &tail[ok_at + ", ok: ".len() + verdict_len..];
    }

    assert!(
        cases.len() >= 8,
        "从前端解析到的用户名样例过少（{} 条），样例表可能被改坏",
        cases.len()
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (name, expected_ok) in &cases {
        // 对账的是 `normalize_username` 而不是 `validate_username`：
        // 前端校验的是**归一后**的值（`normalizeUsername`），所以这里必须
        // 走同一条路径。用 `validate_username` 的话，`  alice  ` 这类样例
        // 两侧结论相反，而测试报"不一致"却指不出到底哪边错了。
        let actual_ok = axum_api::utils::validation::normalize_username(name).is_ok();
        if actual_ok != *expected_ok {
            mismatches.push(format!(
                "{name:?}：前端期望 {expected_ok}，后端实际 {actual_ok}"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "前后端用户名校验判定不一致：\n{}",
        mismatches.join("\n")
    );
}

/// 样例表里必须真的带上一批多字节用户名，否则这条测试形同虚设
///
/// `validate_username` 改成按字符计数这件事，只有在样例里出现
/// 多字节用户名时才会被覆盖到。全是 ASCII 的样例表会让上面那条
/// 在"又改回按字节"的回归下照样绿。
#[test]
fn username_policy_cases_cover_multibyte_names() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("frontend/src/utils/accountRules.ts"))
        .expect("读取 frontend/src/utils/accountRules.ts 失败");

    let body_start = source
        .find("export const USERNAME_POLICY_CASES")
        .expect("前端未导出 USERNAME_POLICY_CASES");
    let body = &source[body_start..];
    let body_end = body
        .find("\n]")
        .expect("USERNAME_POLICY_CASES 数组未正常闭合");
    let body = &body[..body_end];

    // 只统计样例名里"字节数多于字符数"的那些，即确实含多字节字符
    let multibyte: Vec<&str> = body
        .lines()
        .filter_map(|line| {
            let at = line.find("{ name: '")? + "{ name: '".len();
            let end = line[at..].find('\'')? + at;
            let name = &line[at..end];
            (name.len() > name.chars().count()).then_some(name)
        })
        .collect();

    assert!(
        multibyte.len() >= 2,
        "样例表里的多字节用户名只有 {} 条（{multibyte:?}），\
         按字节/按字符的回归将无法被上一条测试发现",
        multibyte.len()
    );
}

/// 账号表单页面必须用共享校验器，不能各写各的
///
/// ## 这条测试为什么存在
///
/// v0.11.0 把口令策略收紧后，`password_policy_agrees_with_the_frontend_copy`
/// 把 `utils/password.ts` 绑到了后端——**但只绑了工具，没绑页面**。
/// 于是注册页与管理员建号对话框整整七版没人碰，各自留着 v0.10 时代的
/// `min: 6`。契约测试一路绿着，而页面在教用户填一个后端不会收的密码。
///
/// 也就是说：真正会漂移的是"页面"，而当时的守卫盯着的是"工具"。
/// 这个洞不补，下一次收紧策略会以同样的方式再来一遍。
#[test]
fn account_forms_use_the_shared_validator() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // 创建账号的两个入口 + 登录页
    let pages = [
        "frontend/src/views/register/index.vue",
        "frontend/src/views/system/user/index.vue",
        "frontend/src/views/login/index.vue",
    ];

    let mut problems: Vec<String> = Vec::new();

    for page in pages {
        let path = root.join(page);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            problems.push(format!("{page}：读不到文件"));
            continue;
        };

        // 注释里写着 `min: 6` 是**应该保留**的修复史记录，
        // 所以只在剥离注释后的真代码里找违规
        let source = script_code_of(&raw);

        if !raw.contains("@/utils/accountRules") {
            problems.push(format!(
                "{page}：没有引用共享校验器 @/utils/accountRules，各写各的规则必然漂移"
            ));
        }

        // 硬编码的口令长度下限：v0.10 策略（至少 6 位）的化石
        for needle in ["min: 6", "min:6", "密码至少 6", "至少 6 个字符"] {
            if source.contains(needle) {
                problems.push(format!(
                    "{page}：出现硬编码 {needle:?}——那是 v0.10 的策略化石，\
                     当前策略是至少 8 位 + 两类字符（见 utils/password.ts）"
                ));
            }
        }

        // 用户名/邮箱字段的手写长度规则
        for needle in ["用户名至少 3", "用户名不能超过 50"] {
            if source.contains(needle) {
                problems.push(format!(
                    "{page}：出现手写规则 {needle:?}，长度应取自 accountRules 的常量"
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "账号表单没有使用共享校验器：\n{}",
        problems.join("\n")
    );
}

/// 共享校验器导出的常量必须与后端常量一致
///
/// 页面上那些 placeholder、"最多 N 个字符"的提示都印着这些数字。
/// 它们离后端有两份拷贝（Rust 与 TS），一旦不一致，
/// 用户看到的提示就在说谎——而这正是本版修的那类缺陷的根因。
#[test]
fn account_rule_constants_agree_with_the_backend() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("frontend/src/utils/accountRules.ts"))
        .expect("读取 frontend/src/utils/accountRules.ts 失败");

    let expect = |const_name: &str, expected: usize| -> Result<(), String> {
        let needle = format!("export const {const_name} = {expected}");
        if source.contains(&needle) {
            Ok(())
        } else {
            Err(format!(
                "accountRules.ts 里没有 `{const_name} = {expected}`；\
                 后端值变了，前端常量要跟着改（页面提示会印着这个数字）"
            ))
        }
    };

    let mut problems = Vec::new();
    for (name, value) in [
        (
            "USERNAME_MIN_LEN",
            axum_api::utils::validation::USERNAME_MIN_LEN,
        ),
        (
            "USERNAME_MAX_LEN",
            axum_api::utils::validation::USERNAME_MAX_LEN,
        ),
        ("IDENTIFIER_MAX_LEN", 255),
    ] {
        if let Err(msg) = expect(name, value) {
            problems.push(msg);
        }
    }

    assert!(
        problems.is_empty(),
        "前后端常量不一致：\n{}",
        problems.join("\n")
    );
}
