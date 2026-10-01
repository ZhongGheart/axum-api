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

/// 契约测试：每个 `/api/admin/*` handler 都必须声明权限码。
///
/// 防止将来新增管理接口时漏接 `perm.require(...)`，从而绕过授权体系。
#[test]
fn every_admin_handler_declares_a_permission_code() {
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

            let has_require = body.contains("perm.require(permission::");
            if !has_require {
                offenders.push(format!(
                    "{}::{name} 未调用 perm.require(permission::…)",
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
        "以下管理接口缺少权限码校验:\n  {}",
        offenders.join("\n  ")
    );
}
