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

use crate::config::Config;
use crate::controller::{auth, demo, dict, menu, rbac, role, user};
use crate::error::AppError;
use crate::middleware::auth::auth_middleware;
use crate::middleware::rate_limit::rate_limit_middleware;
use crate::middleware::request_id::request_id_middleware;
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
    #[allow(dead_code)]
    pub rbac_service: RbacService,
    pub jwt_util: Arc<JwtUtil>,
    pub redis_client: Arc<RedisClient>,
    pub menu_repo: MenuRepository,
    pub dict_repo: DictRepository,
}

/// 构建应用路由
pub async fn create_router(config: Config) -> Result<Router, AppError> {
    // ── 初始化数据库连接池（带 PoolOptions 配置） ──────────────
    use sqlx::postgres::PgPoolOptions;
    let pool = PgPoolOptions::new()
        .max_connections(config.pool.max_size)
        .acquire_timeout(std::time::Duration::from_secs(
            config.pool.connect_timeout_seconds,
        ))
        .connect(&config.database_url)
        .await
        .map_err(|e| AppError::InternalServerError(format!("数据库连接失败: {e}")))?;

    // ── 初始化 Redis 客户端 ────────────────────────────────────
    let redis_client = Arc::new(
        RedisClient::new(&config.redis)
            .await
            .map_err(|e| {
                tracing::warn!("Redis 连接失败，限流和黑名单功能将不可用: {e}");
                e
            })?,
    );

    // ── 初始化各层 ──────────────────────────────────────────────
    let jwt_util = Arc::new(JwtUtil::new(&config.jwt_secret));

    let user_repo = UserRepository::new(pool.clone());
    let role_repo = RoleRepository::new(pool.clone());
    let menu_repo = MenuRepository::new(pool.clone());
    let dict_repo = DictRepository::new(pool.clone(), Some(redis_client.as_ref().clone()));
    let auth_service = AuthService::new(
        user_repo,
        role_repo.clone(),
        jwt_util.as_ref().clone(),
        config.jwt_expiration_seconds,
    );
    let rbac_service = RbacService::new(role_repo, pool.clone());

    rbac_service.init_defaults().await?;

    let state = AppState {
        auth_service,
        rbac_service,
        jwt_util: Arc::clone(&jwt_util),
        redis_client: Arc::clone(&redis_client),
        menu_repo,
        dict_repo,
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
            auth_middleware,
        ));

    // ── 需要 admin 角色的路由 ──────────────────────────────────
    let admin_routes = Router::new()
        .route("/api/admin/test", get(rbac::admin_test))
        .route("/api/admin/users", get(user::list_users).post(user::create_user))
        .route("/api/admin/users/{id}", axum::routing::put(user::update_user).delete(user::delete_user))
        .route("/api/admin/roles", get(role::list_roles).post(role::create_role))
        .route("/api/admin/roles/{id}", axum::routing::put(role::update_role).delete(role::delete_role))
        .route("/api/admin/users/{id}/roles", get(role::get_user_roles).post(role::assign_user_role))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        // auth_middleware 先运行（外层），解析 JWT 注入 AuthenticatedUser
        // require_role 再运行（内层），读取 AuthenticatedUser 校验角色
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 菜单管理路由（仅 admin） ────────────────────────────
    let menu_routes = Router::new()
        .route("/api/admin/menus", get(menu::list_menus).post(menu::create_menu))
        .route("/api/admin/menus/{id}", axum::routing::put(menu::update_menu).delete(menu::delete_menu))
        .route("/api/admin/roles/{id}/menus", axum::routing::put(menu::assign_role_menus))
        .route_layer(middleware::from_fn(move |req, next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // ── 数据字典路由（仅 admin） ────────────────────────────
    let dict_routes = Router::new()
        .route("/api/admin/dict/types", get(dict::list_types).post(dict::create_type))
        .route("/api/admin/dict/types/{id}", axum::routing::put(dict::update_type).delete(dict::delete_type))
        .route("/api/admin/dict/items", get(dict::list_items).post(dict::create_item))
        .route("/api/admin/dict/items/{id}", axum::routing::put(dict::update_item).delete(dict::delete_item))
        .route("/api/admin/dict/{code}/items", get(dict::get_items_by_code))
        .route("/api/admin/dict/cached", get(dict::list_all_cached))
        .route("/api/admin/dict/refresh", post(dict::refresh_cache))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // ── 能力测试路由（仅 admin） ────────────────────────────
    let demo_routes = Router::new()
        .route("/api/admin/export/users", axum::routing::get(demo::export_users))
        .route("/api/admin/validate", axum::routing::post(demo::validate_test))
        .route("/api/admin/audit-logs", axum::routing::get(demo::list_audit_logs))
        .route("/api/admin/logs/audit/export", axum::routing::get(demo::export_audit_logs))
        .route("/api/admin/users/batch-delete", axum::routing::post(user::batch_delete_users))
        .route("/api/admin/users/{id}/status", axum::routing::put(user::toggle_user_status))
        .route("/api/admin/users/{id}/reset-password", axum::routing::post(user::reset_user_password))
        .route("/api/admin/users/{id}/roles", axum::routing::put(user::assign_user_roles))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 合并所有路由并应用全局中间件 ──────────────────────────
    let rate_limit_state = (Arc::clone(&redis_client), Arc::new(config.rate_limit));
    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(admin_routes)
        .merge(demo_routes)
        .merge(menu_routes)
        .merge(dict_routes)
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
