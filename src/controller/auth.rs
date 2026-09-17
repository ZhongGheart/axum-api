//! 认证控制器
//!
//! 处理认证相关的 HTTP 请求，包括注册、登录、获取当前用户信息、登出。

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::client_ip::ClientIp;
use crate::model::{ApiResponse, LoginRequest, LoginResponse, RegisterRequest, UserInfo};
use crate::router::AppState;

/// POST /api/auth/register — 用户注册
#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "认证",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "注册成功", body = ApiResponse<UserInfo>),
        (status = 400, description = "参数不合法"),
        (status = 409, description = "用户名或邮箱已存在"),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user_info = state.auth_service.register(req).await?;
    Ok(Json(ApiResponse::success(user_info)))
}

/// POST /api/auth/login — 用户登录
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "认证",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = ApiResponse<LoginResponse>),
        (status = 401, description = "用户名或密码错误"),
        (status = 429, description = "登录失败次数过多，已临时锁定"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    client_ip: ClientIp,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let login_resp = state
        .auth_service
        .login(req, &state.redis_client, &client_ip.0)
        .await?;
    Ok(Json(ApiResponse::success(login_resp)))
}

/// GET /api/auth/me — 获取当前用户信息（含角色列表）
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户信息（含角色列表）", body = ApiResponse<UserInfo>))
)]
pub async fn me(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user = state
        .auth_service
        .user_repo
        .find_by_id(auth_user.user_id)
        .await?;
    let roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(auth_user.user_id)
        .await?;
    let user_info = crate::model::UserInfo::new(user, roles);
    Ok(Json(ApiResponse::success(user_info)))
}

/// POST /api/auth/logout — 用户登出（仅注销当前令牌）
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "登出成功（仅当前令牌失效）", body = ApiResponse<String>))
)]
pub async fn logout(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state
        .auth_service
        .logout(
            &state.redis_client,
            &auth_user.token_jti,
            auth_user.token_exp,
        )
        .await?;
    Ok(Json(ApiResponse::success("登出成功")))
}

/// 健康检查响应体
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthPayload {
    /// 总体状态：`ok` / `degraded`
    pub status: &'static str,
    /// 数据库连通性：`up` / `down`
    pub database: &'static str,
    /// Redis 连通性：`up` / `down`
    pub redis: &'static str,
}

/// GET /api/health — 健康检查
///
/// 真实探测数据库与 Redis：任一依赖不可用时返回 503，
/// 以便容器编排与负载均衡摘除该实例。
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "系统",
    responses(
        (status = 200, description = "服务正常", body = ApiResponse<HealthPayload>),
        (status = 503, description = "依赖服务不可用", body = ApiResponse<HealthPayload>),
    )
)]
pub async fn health(State(state): State<AppState>) -> axum::response::Response {
    let database_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.db_pool.writer())
        .await
        .is_ok();
    let redis_ok = state.redis_client.ping().await.is_ok();
    let healthy = database_ok && redis_ok;

    let payload = HealthPayload {
        status: if healthy { "ok" } else { "degraded" },
        database: if database_ok { "up" } else { "down" },
        redis: if redis_ok { "up" } else { "down" },
    };

    let (status, message) = if healthy {
        (StatusCode::OK, "服务运行正常")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "依赖服务不可用")
    };

    (
        status,
        Json(ApiResponse {
            code: status.as_u16(),
            message: message.to_string(),
            data: Some(payload),
        }),
    )
        .into_response()
}
