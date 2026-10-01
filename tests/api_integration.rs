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
use tower::ServiceExt;

use axum_api::config::{Config, DatabaseConfig, RateLimitConfig, RedisConfig, SecurityConfig};
use axum_api::router::create_router;

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
        migrate_on_startup: true,
    }
}

/// 每个用例构建独立路由
///
/// 连接管理器（Redis/DB）绑定创建它的 Tokio runtime，
/// 而 `#[tokio::test]` 每个用例都会新建 runtime，因此不能跨用例共享路由。
async fn app() -> Router {
    create_router(test_config(1_000))
        .await
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
    let strict = create_router(test_config(3)).await.unwrap();
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

    let token = login_token(&app, &username, "user1234").await;

    // 普通用户可读取字典（通用展示数据）
    let (dict_status, _) = send(
        &app,
        request("GET", "/api/dict/status/items", Some(&token), None),
    )
    .await;
    assert_ne!(dict_status, StatusCode::FORBIDDEN, "字典读取不应要求 admin");

    // 但不能访问管理接口
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

    let user = login_token(&app, &username, "user1234").await;
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

    let token = login_token(&app, &username, "user1234").await;
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

    // 用户表单目前只允许分配内置角色（ASSIGNABLE_ROLES），因此直接写库构造占用
    let username = unique("user_inuse");
    let (status, created) = send(
        &app,
        request(
            "POST",
            "/api/admin/users",
            Some(&token),
            Some(json!({
                "username": username,
                "email": format!("{username}@example.com"),
                "password": "user1234",
                "role": "user"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "创建测试用户失败: {created}");
    let user_id = uuid::Uuid::parse_str(created["data"]["id"].as_str().unwrap()).unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(&pool().await)
        .await
        .expect("构造角色占用失败");

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

    // 清理：角色无用户占用，可直接删；删目录会级联清掉页面与按钮
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

            // 守卫是签名里的类型化提取器参数，在提取阶段完成校验
            let has_guard = signature.contains("_perm: Perm");
            if !has_guard {
                offenders.push(format!(
                    "{}::{name} 未声明类型化权限码守卫（_perm: Perm…）",
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
