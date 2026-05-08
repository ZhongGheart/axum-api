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
use crate::controller::auth;
use crate::error::AppError;
use crate::middleware::auth::auth_middleware;
use crate::repository::user::UserRepository;
use crate::service::auth::AuthService;
use crate::utils::jwt::JwtUtil;

/// 应用共享状态
#[derive(Debug, Clone)]
pub struct AppState {
    /// 认证服务
    pub auth_service: AuthService,
    /// JWT 工具（Arc 包装以便在中间件中共享）
    pub jwt_util: Arc<JwtUtil>,
}

/// 构建应用路由
///
/// 1. 公开路由（无需认证）
/// 2. 需要认证的路由（应用 JWT 中间件）
pub fn create_router(config: Config) -> Result<Router, AppError> {
    // ── 初始化数据库连接池 ──────────────────────────────────────
    let pool = sqlx::PgPool::connect_lazy(&config.database_url)
        .map_err(|e| AppError::InternalServerError(format!("数据库连接失败: {e}")))?;

    // ── 初始化各层 ──────────────────────────────────────────────
    let jwt_util = Arc::new(JwtUtil::new(&config.jwt_secret));

    let user_repo = UserRepository::new(pool);
    let auth_service = AuthService::new(
        user_repo,
        jwt_util.as_ref().clone(),
        config.jwt_expiration_seconds,
    );

    let state = AppState {
        auth_service,
        jwt_util: Arc::clone(&jwt_util),
    };

    // ── 配置 CORS ──────────────────────────────────────────────
    let cors = CorsLayer::new()
        .allow_origin(
            config
                .cors_allowed_origins
                .iter()
                .map(|origin| origin.parse().unwrap())
                .collect::<Vec<_>>(),
        )
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    // ── 公开路由（无需认证） ─────────────────────────────────────
    let public_routes = Router::new()
        .route("/api/health", get(auth::health))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login));

    // ── 需要认证的路由 ──────────────────────────────────────────
    let protected_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── 合并所有路由 ────────────────────────────────────────────
    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    Ok(app)
}
