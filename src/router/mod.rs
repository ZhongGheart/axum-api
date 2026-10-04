//! 路由层
//!
//! 注册所有 API 路由分组，配置全局中间件。

use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post, put},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
// Swagger UI 通过前端 iframe + CDN 渲染

use crate::config::{AuditLogConfig, Config, UploadConfig};
use crate::controller::{auth, demo, dict, menu, monitor, rbac, role, setting, user};
use crate::docs::swagger_ui_handler;
use crate::error::AppError;
use crate::middleware::api_metrics::{api_metrics_mw, MetricsCollector};
use crate::middleware::audit_log::audit_log_middleware;
use crate::middleware::auth::auth_middleware;
use crate::middleware::rate_limit::rate_limit_middleware;
use crate::middleware::request_id::request_id_middleware;
use crate::repository::audit_log::AuditLogRepository;
use crate::repository::db::DatabasePool;
use crate::repository::dict::DictRepository;
use crate::repository::menu::MenuRepository;
use crate::repository::role::RoleRepository;
use crate::repository::setting::SettingRepository;
use crate::repository::user::UserRepository;
use crate::service::auth::AuthService;
use crate::service::rbac::RbacService;
use crate::service::setting::SettingService;
use crate::utils::jwt::JwtUtil;
use crate::utils::redis::RedisClient;
use crate::utils::upload;

/// 应用共享状态
#[derive(Debug, Clone)]
pub struct AppState {
    pub auth_service: AuthService,
    pub jwt_util: Arc<JwtUtil>,
    pub redis_client: Arc<RedisClient>,
    pub menu_repo: MenuRepository,
    pub dict_repo: DictRepository,
    pub audit_log_repo: AuditLogRepository,
    pub db_pool: DatabasePool,
    pub metrics_collector: Arc<MetricsCollector>,
    /// 审计日志保留策略（v0.14.0）
    ///
    /// 必须放在 `AppState` 而不是让控制器去读环境变量：接口要**如实报告**
    /// 当前部署的真实保留天数，而重新读一次环境变量在测试里可能被改过、
    /// 或在多副本部署下与实际跑的清理任务不一致。
    pub audit_log_config: AuditLogConfig,
    /// 头像上传配置（v0.20.0）
    pub upload_config: UploadConfig,
    /// 系统参数服务（v0.22.0）
    ///
    /// 口令策略与登录防护阈值从这里读，**不再**从 `Config` 读常量——
    /// 那样改一次要重启进程，且容器编排里根本没法改。
    pub setting_service: SettingService,
}

/// 构建应用路由
///
/// 同时返回 `AppState`：后台任务（指标 flush、审计日志清理）由 `main` 启动，
/// 它们属于进程生命周期而非应用组装，因此不放在这里 spawn——
/// 否则集成测试反复构建应用会留下一堆任务去打共享测试库。
pub async fn create_router(config: Config) -> Result<(Router, AppState), AppError> {
    // ── 初始化读写分离数据库连接池 ────────────────────────
    let db_pool = DatabasePool::new(&config.database).await?;

    // ── 启动时执行数据库迁移（空库自动建表） ──────────────────
    if config.migrate_on_startup {
        db_pool.run_migrations().await?;
        tracing::info!("数据库迁移已应用");
    }

    let pool = db_pool.writer(); // 主库用于初始化

    // ── 初始化 Redis 客户端 ────────────────────────────────────
    let redis_client = Arc::new(RedisClient::new(&config.redis).await.map_err(|e| {
        tracing::warn!("Redis 连接失败，限流和黑名单功能将不可用: {e}");
        e
    })?);

    // ── 初始化各层 ──────────────────────────────────────────────
    let jwt_util = Arc::new(JwtUtil::new(&config.jwt_secret));
    // 指标聚合到 Redis：多副本共享同一份计数，重启与"重置"都不再只作用于本进程
    let metrics_collector = Arc::new(MetricsCollector::new(
        Arc::clone(&redis_client),
        config.metrics.clone(),
    ));

    let user_repo = UserRepository::new(pool.clone());
    let role_repo = RoleRepository::new(pool.clone());
    let menu_repo = MenuRepository::new(pool.clone());
    let dict_repo = DictRepository::new(pool.clone(), Some(redis_client.as_ref().clone()));
    let audit_log_repo = AuditLogRepository::new(pool.clone());
    // 参数仓储与字典仓储同构：DB 权威 + Redis 缓存
    let setting_repo = SettingRepository::new(pool.clone(), Some(redis_client.as_ref().clone()));
    let setting_service = SettingService::new(
        setting_repo,
        // 部署配置作为"没人改过时的生效值"；管理员显式改过则以参数表为准。
        // 理由见 `service::setting::EnvFallbacks`。
        crate::service::setting::EnvFallbacks {
            login_max_failures: config.security.login_max_failures,
            login_failure_window_seconds: config.security.login_failure_window_seconds,
        },
    );
    let auth_service = AuthService::new(
        user_repo,
        role_repo.clone(),
        jwt_util.as_ref().clone(),
        config.jwt_expiration_seconds,
        audit_log_repo.clone(),
        setting_service.clone(),
    );
    let rbac_service = RbacService::new(pool.clone());

    rbac_service.init_defaults().await?;

    let state = AppState {
        auth_service,
        jwt_util: Arc::clone(&jwt_util),
        redis_client: Arc::clone(&redis_client),
        menu_repo,
        dict_repo,
        audit_log_repo,
        db_pool,
        metrics_collector: metrics_collector.clone(),
        audit_log_config: config.audit_log.clone(),
        upload_config: config.upload.clone(),
        setting_service: setting_service.clone(),
    };

    // 上传目录必须在挂 ServeDir 之前存在：ServeDir 只在请求到达时才去解析路径，
    // 目录缺失时的表现是首次访问拿到 404，而不是启动期报错——
    // 那意味着"部署漏了挂卷"要等到第一个用户传头像时才被发现。
    if let Err(e) = tokio::fs::create_dir_all(&config.upload.dir).await {
        return Err(AppError::InternalServerError(format!(
            "创建上传目录 {} 失败: {e}",
            config.upload.dir
        )));
    }

    // ── 配置 CORS ──────────────────────────────────────────────
    let mut cors = CorsLayer::new()
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let valid_origins: Vec<_> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| {
            if origin == "*" {
                None
            } else {
                origin.parse::<axum::http::HeaderValue>().ok()
            }
        })
        .collect();

    if !valid_origins.is_empty() {
        cors = cors.allow_origin(valid_origins);
    } else if config.cors_allowed_origins.iter().any(|o| o == "*") {
        cors = cors.allow_origin(tower_http::cors::Any);
    }

    // ── 公开路由 ───────────────────────────────────────────────
    let public_routes = Router::new()
        .route("/api/health", get(auth::health))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        // 口令策略对**未登录**页面公开：注册页需要它来提示要求，
        // 否则用户只能在提交失败后从报错里反推。
        // 返回体只含四条"设口令时必须知道"的规则，不含锁定阈值。
        .route(
            "/api/settings/password-policy",
            get(setting::public_password_policy),
        );

    // ── 系统参数管理路由 ────────────────────────────────────
    let setting_routes = Router::new()
        .route("/api/admin/settings", get(setting::list_settings))
        // 静态段必须排在 `{key}` 之前：`refresh-cache` 若被当成 key，
        // 下一个请求就变成"没有名为 refresh-cache 的系统参数"（400）。
        .route(
            "/api/admin/settings/refresh-cache",
            post(setting::refresh_cache),
        )
        .route(
            "/api/admin/settings/{key}",
            axum::routing::put(setting::update_setting),
        )
        .route(
            "/api/admin/settings/{key}/reset",
            axum::routing::post(setting::reset_setting),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 需要认证的路由 ─────────────────────────────────────────
    let protected_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/logout", post(auth::logout))
        // 自助改密。挂在受保护路由内，且**刻意不放进 audit_log_middleware
        // 之外的位置**：改密要落审计（走中间件），同时它是"受限令牌"
        // 唯一被放行的写接口
        .route("/api/auth/password", put(auth::change_password))
        // 自助改资料（展示名 / 头像）。
        // **刻意不在受限令牌白名单里**：待改密的用户先改口令。
        // 那条白名单（middleware/auth.rs 的 pwd_stale 分支）只放行
        // password / logout / me，加 profile 就等于允许一个尚未确认凭据的
        // 会话去写用户数据。
        .route("/api/auth/profile", put(auth::update_profile))
        // 头像上传。与 profile 一样刻意不在受限令牌白名单里：
        // 待改密的用户先改口令。
        .route("/api/auth/profile/avatar", post(auth::upload_avatar))
        // 自助会话管理（v0.23.0）。
        //
        // `revoke-others` 这个静态段**必须**排在 `{jti}/revoke` 之前：
        // 排后面的话 `revoke-others` 会被当成一个 jti 走进单会话吊销，
        // 然后以"不是合法的 UUID"400——一个看起来像参数错误、
        // 实际是路由根本没匹配上的响应。
        .route("/api/auth/sessions", get(auth::my_sessions))
        .route(
            "/api/auth/sessions/revoke-others",
            post(auth::revoke_my_other_sessions),
        )
        .route(
            "/api/auth/sessions/{jti}/revoke",
            post(auth::revoke_my_session),
        )
        // 当前用户的导航菜单：前端据此动态生成路由与侧栏
        .route("/api/auth/menus", get(crate::controller::menu::my_menus))
        // 当前用户的权限码：前端 v-permission 据此判定按钮级权限
        .route(
            "/api/auth/permissions",
            get(crate::controller::menu::my_permissions),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 系统管理路由（权限码闸门，无角色闸门）────────────────
    //
    // v0.5.0 PR-3 起不再叠加 `require_role("admin")`：
    // "能不能进管理区"完全由各 handler 的类型化权限码守卫决定，
    // 持有 `system:user:list` 这类码的自定义角色即可管理对应模块。
    // 授权下界（能授予的 ⊆ 自己已持有的）见 `middleware::permission`。
    let admin_routes = Router::new()
        .route("/api/admin/test", get(rbac::admin_test))
        .route(
            "/api/admin/users",
            get(user::list_users).post(user::create_user),
        )
        .route(
            "/api/admin/users/{id}",
            axum::routing::put(user::update_user).delete(user::delete_user),
        )
        .route(
            "/api/admin/roles",
            get(role::list_roles).post(role::create_role),
        )
        .route(
            "/api/admin/roles/{id}",
            axum::routing::put(role::update_role).delete(role::delete_role),
        )
        .route(
            "/api/admin/users/{user_id}/roles",
            get(role::get_user_roles).post(role::assign_user_role),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 菜单管理路由 ────────────────────────────────────────
    let menu_routes = Router::new()
        .route(
            "/api/admin/menus",
            get(menu::list_menus).post(menu::create_menu),
        )
        // 静态段优先于 `{id}`：matchit 会先匹配字面量，`diagnostics` 不会被
        // 当成菜单 id 去喂给 `update_menu`/`delete_menu`。
        .route("/api/admin/menus/diagnostics", get(menu::menu_diagnostics))
        .route(
            "/api/admin/menus/{id}",
            axum::routing::put(menu::update_menu).delete(menu::delete_menu),
        )
        .route(
            "/api/admin/menus/{id}/restore-permission",
            axum::routing::post(menu::restore_menu_permission),
        )
        .route(
            "/api/admin/roles/{role_id}/menus",
            axum::routing::put(menu::assign_role_menus),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 数据字典管理路由 ────────────────────────────────────
    let dict_routes = Router::new()
        .route(
            "/api/admin/dict/types",
            get(dict::list_types).post(dict::create_type),
        )
        .route(
            "/api/admin/dict/types/{id}",
            axum::routing::put(dict::update_type).delete(dict::delete_type),
        )
        .route(
            "/api/admin/dict/items",
            get(dict::list_items).post(dict::create_item),
        )
        .route(
            "/api/admin/dict/items/{id}",
            axum::routing::put(dict::update_item).delete(dict::delete_item),
        )
        .route("/api/admin/dict/cached", get(dict::list_all_cached))
        .route("/api/admin/dict/refresh", post(dict::refresh_cache))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 数据字典读取（任意已登录用户） ──────────────────────
    // 字典是通用展示数据；要求 admin 会让所有非管理页面的 DictSelect 直接 403
    let dict_read_routes = Router::new()
        .route("/api/dict/{code}/items", get(dict::get_items_by_code))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 能力测试与用户写操作路由 ────────────────────────────
    let demo_routes = Router::new()
        .route(
            "/api/admin/export/users",
            axum::routing::get(demo::export_users),
        )
        .route(
            "/api/admin/validate",
            axum::routing::post(demo::validate_test),
        )
        .route(
            "/api/admin/audit-logs",
            axum::routing::get(demo::list_audit_logs),
        )
        // 必须排在 `/api/admin/audit-logs` 之后、且两者路径不冲突：
        // axum 0.8 的 `{id}` 语法已不支持通配尾段，这里的路径是静态的
        .route(
            "/api/admin/audit-logs/retention",
            axum::routing::get(demo::audit_log_retention),
        )
        .route(
            "/api/admin/logs/audit/export",
            axum::routing::get(demo::export_audit_logs),
        )
        .route(
            "/api/admin/users/batch-delete",
            axum::routing::post(user::batch_delete_users),
        )
        // CSV 批量导入。必须排在 `/api/admin/users/{id}` 之前：
        // matchit 对同层字面量优先，但留在这里是为了让"哪些是字面量段"
        // 一眼可辨——`import` 若被当成 id，下一个请求就变成 400 查 UUID 失败。
        .route(
            "/api/admin/users/import",
            axum::routing::post(user::import_users),
        )
        .route(
            "/api/admin/users/{id}/status",
            axum::routing::put(user::toggle_user_status),
        )
        // 解锁被登录爆破防护临时锁定的账号。挂在 /api/admin 分组内，
        // 因而自动继承该分组的 require_role 闸门与守卫检查
        // （every_admin_handler_declares_a_permission_guard 会自动纳入它）。
        .route(
            "/api/admin/users/{id}/unlock",
            axum::routing::post(user::unlock_user),
        )
        .route(
            "/api/admin/users/{id}/sessions",
            axum::routing::get(user::list_user_sessions),
        )
        // 单会话吊销。与"吊销全部会话"是两条独立路径：
        // 后者走 user_revoked_before 的时间戳机制，前者走 jti 黑名单。
        .route(
            "/api/admin/users/{id}/sessions/{jti}/revoke",
            axum::routing::post(user::revoke_user_session),
        )
        .route(
            "/api/admin/users/{id}/reset-password",
            axum::routing::post(user::reset_user_password),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 系统监控路由 ────────────────────────────────────────
    let monitor_routes = Router::new()
        .route("/api/admin/monitor/system", get(monitor::system_info))
        .route("/api/admin/monitor/api-metrics", get(monitor::api_metrics))
        .route("/api/admin/monitor/alerts", get(monitor::alerts))
        .route(
            "/api/admin/monitor/metrics/reset",
            post(monitor::reset_metrics),
        )
        .route(
            "/api/admin/monitor/system/export",
            get(monitor::export_system),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 合并所有路由并应用全局中间件 ──────────────────────────
    let rate_limit_state = (
        Arc::clone(&redis_client),
        Arc::new(config.rate_limit.clone()),
        config.security.trust_proxy_headers,
    );
    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(admin_routes)
        .merge(demo_routes)
        .merge(menu_routes)
        .merge(dict_routes)
        .merge(dict_read_routes)
        .merge(monitor_routes)
        .merge(setting_routes)
        // 上传目录的静态访问。**刻意挂在鉴权之外**：
        // 头像要能被 `<img src>` 直接取，而 `<img>` 无法附带 Authorization 头。
        // 换来的是这些图是公开可读的——头像本来就在用户列表页展示，
        // 这与"用户列表要登录才能看"不是同一个承诺。
        .nest_service(
            upload::UPLOAD_URL_PREFIX,
            tower_http::services::ServeDir::new(&config.upload.dir),
        )
        // OpenAPI JSON 端点
        .route(
            "/api/openapi.json",
            axum::routing::get(|| async { axum::Json(crate::docs::openapi_json()) }),
        )
        // Swagger UI HTML 页面（同源服务，避免 iframe 跨域限制）
        .route(
            "/api/swagger-ui/{*path}",
            axum::routing::get(swagger_ui_handler),
        )
        // API 性能追踪中间件（记录每个接口的耗时/报错）
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_metrics_mw,
        ))
        // 全局中间件：限流（最外层）
        .layer(middleware::from_fn_with_state(
            rate_limit_state,
            rate_limit_middleware,
        ))
        // 全局中间件：请求 ID
        .layer(middleware::from_fn(request_id_middleware))
        // 全局中间件：HTTP 追踪日志
        .layer(TraceLayer::new_for_http())
        // CORS
        .layer(cors)
        // 注入共享状态
        .with_state(state.clone());

    Ok((app, state))
}
