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
use crate::controller::{auth, rbac};
use crate::error::AppError;
use crate::middleware::auth::auth_middleware;
use crate::repository::role::RoleRepository;
use crate::repository::user::UserRepository;
use crate::service::auth::AuthService;
use crate::service::rbac::RbacService;
use crate::utils::jwt::JwtUtil;

/// 应用共享状态
#[derive(Debug, Clone)]
pub struct AppState {
    /// 认证服务
    pub auth_service: AuthService,
    /// RBAC 权限服务（供路由注册时初始化用）
    #[allow(dead_code)]
    pub rbac_service: RbacService,
    /// JWT 工具（Arc 包装以便在中间件中共享）
    pub jwt_util: Arc<JwtUtil>,
}

/// 构建应用路由
///
/// 1. 公开路由（无需认证）
/// 2. 需要认证的路由（应用 JWT 中间件）
/// 3. 需要特定角色权限的路由（叠加角色中间件）
pub async fn create_router(config: Config) -> Result<Router, AppError> {
    // ── 初始化数据库连接池（异步，启动时即检测连通性） ────────────
    let pool = sqlx::PgPool::connect(&config.database_url)
        .await
        .map_err(|e| AppError::InternalServerError(format!("数据库连接失败: {e}")))?;

    // ── 初始化各层 ──────────────────────────────────────────────
    let jwt_util = Arc::new(JwtUtil::new(&config.jwt_secret));

    let user_repo = UserRepository::new(pool.clone());
    let role_repo = RoleRepository::new(pool.clone());
    let auth_service = AuthService::new(
        user_repo,
        role_repo.clone(),
        jwt_util.as_ref().clone(),
        config.jwt_expiration_seconds,
    );
    let rbac_service = RbacService::new(role_repo, pool.clone());

    // ── 初始化 RBAC 种子数据（幂等） ───────────────────────────
    rbac_service.init_defaults().await?;

    let state = AppState {
        auth_service,
        rbac_service,
        jwt_util: Arc::clone(&jwt_util),
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

    // ── 公开路由（无需认证） ─────────────────────────────────────
    let public_routes = Router::new()
        .route("/api/health", get(auth::health))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login));

    // ── 需要认证的路由（JWT 鉴权） ──────────────────────────────
    let protected_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 需要 admin 角色的路由（认证 + 角色拦截） ─────────────────
    let admin_routes = Router::new()
        .route("/api/admin/test", get(rbac::admin_test))
        .route_layer(middleware::from_fn(move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| {
            async move { crate::middleware::auth::require_role("admin", req, next).await }
        }));

    // ── 合并所有路由 ────────────────────────────────────────────
    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(admin_routes)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    Ok(app)
}
