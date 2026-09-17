//! 路由层
//!
//! 注册所有 API 路由分组，配置全局中间件。

use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
// Swagger UI 通过前端 iframe + CDN 渲染

use crate::config::Config;
use crate::controller::{auth, demo, dict, menu, monitor, rbac, role, user};
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
use crate::repository::user::UserRepository;
use crate::service::auth::AuthService;
use crate::service::rbac::RbacService;
use crate::utils::jwt::JwtUtil;
use crate::utils::redis::RedisClient;

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
}

/// 构建应用路由
pub async fn create_router(config: Config) -> Result<Router, AppError> {
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
    let metrics_collector = Arc::new(MetricsCollector::new());

    let user_repo = UserRepository::new(pool.clone());
    let role_repo = RoleRepository::new(pool.clone());
    let menu_repo = MenuRepository::new(pool.clone());
    let dict_repo = DictRepository::new(pool.clone(), Some(redis_client.as_ref().clone()));
    let audit_log_repo = AuditLogRepository::new(pool.clone());
    let auth_service = AuthService::new(
        user_repo,
        role_repo.clone(),
        jwt_util.as_ref().clone(),
        config.jwt_expiration_seconds,
        config.security.login_max_failures,
        config.security.login_failure_window_seconds,
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
    };

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
        .route("/api/auth/login", post(auth::login));

    // ── 需要认证的路由 ─────────────────────────────────────────
    let protected_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/logout", post(auth::logout))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 需要 admin 角色的路由 ──────────────────────────────────
    let admin_routes = Router::new()
        .route("/api/admin/test", get(rbac::admin_test))
        .route("/api/admin/users", get(user::list_users).post(user::create_user))
        .route("/api/admin/users/{id}", axum::routing::put(user::update_user).delete(user::delete_user))
        .route("/api/admin/roles", get(role::list_roles).post(role::create_role))
        .route("/api/admin/roles/{id}", axum::routing::put(role::update_role).delete(role::delete_role))
        .route("/api/admin/users/{user_id}/roles", get(role::get_user_roles).post(role::assign_user_role))
        .layer(middleware::from_fn_with_state(state.clone(), audit_log_middleware))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // ── 菜单管理路由（仅 admin） ────────────────────────────
    let menu_routes = Router::new()
        .route(
            "/api/admin/menus",
            get(menu::list_menus).post(menu::create_menu),
        )
        .route(
            "/api/admin/menus/{id}",
            axum::routing::put(menu::update_menu).delete(menu::delete_menu),
        )
        .route(
            "/api/admin/roles/{role_id}/menus",
            axum::routing::put(menu::assign_role_menus),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_log_middleware,
        ))
        .route_layer(middleware::from_fn(move |req, next| async move {
            crate::middleware::auth::require_role("admin", req, next).await
        }))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 数据字典路由（仅 admin） ────────────────────────────
    let dict_routes = Router::new()
        .route("/api/admin/dict/types", get(dict::list_types).post(dict::create_type))
        .route("/api/admin/dict/types/{id}", axum::routing::put(dict::update_type).delete(dict::delete_type))
        .route("/api/admin/dict/items", get(dict::list_items).post(dict::create_item))
        .route("/api/admin/dict/items/{id}", axum::routing::put(dict::update_item).delete(dict::delete_item))
        .route("/api/admin/dict/cached", get(dict::list_all_cached))
        .route("/api/admin/dict/refresh", post(dict::refresh_cache))
        .layer(middleware::from_fn_with_state(state.clone(), audit_log_middleware))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

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

    // ── 能力测试路由（仅 admin） ────────────────────────────
    let demo_routes = Router::new()
        .route("/api/admin/export/users", axum::routing::get(demo::export_users))
        .route("/api/admin/validate", axum::routing::post(demo::validate_test))
        .route("/api/admin/audit-logs", axum::routing::get(demo::list_audit_logs))
        .route("/api/admin/logs/audit/export", axum::routing::get(demo::export_audit_logs))
        .route("/api/admin/users/batch-delete", axum::routing::post(user::batch_delete_users))
        .route("/api/admin/users/{id}/status", axum::routing::put(user::toggle_user_status))
        .route("/api/admin/users/{id}/reset-password", axum::routing::post(user::reset_user_password))
        .layer(middleware::from_fn_with_state(state.clone(), audit_log_middleware))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // ── 系统监控路由（仅 admin） ────────────────────────────
    let monitor_routes = Router::new()
        .route("/api/admin/monitor/system", get(monitor::system_info))
        .route("/api/admin/monitor/api-metrics", get(monitor::api_metrics))
        .route("/api/admin/monitor/alerts", get(monitor::alerts))
        .route("/api/admin/monitor/metrics/reset", post(monitor::reset_metrics))
        .route("/api/admin/monitor/system/export", get(monitor::export_system))
        .layer(middleware::from_fn_with_state(state.clone(), audit_log_middleware))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

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
        .with_state(state);

    Ok(app)
}
