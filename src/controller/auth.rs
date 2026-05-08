//! 认证控制器
//!
//! 处理认证相关的 HTTP 请求，包括注册、登录、获取当前用户信息。
//! 控制器通过 `State<AppState>` 获取服务层实例。

use axum::{extract::State, Json};

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::model::{ApiResponse, LoginRequest, LoginResponse, RegisterRequest, UserInfo};
use crate::router::AppState;

/// POST /api/auth/register — 用户注册
///
/// 请求体：`RegisterRequest`（username, email, password）
/// 响应：用户信息（不含密码）
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user_info = state.auth_service.register(req).await?;
    Ok(Json(ApiResponse::success(user_info)))
}

/// POST /api/auth/login — 用户登录
///
/// 请求体：`LoginRequest`（username, password）
/// 响应：JWT 令牌
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let login_resp = state.auth_service.login(req).await?;
    Ok(Json(ApiResponse::success(login_resp)))
}

/// GET /api/auth/me — 获取当前用户信息
///
/// 需要 JWT 认证（通过中间件）。
/// 从 `AuthenticatedUser` 提取器中自动获取当前用户信息。
pub async fn me(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user_info = state.auth_service.get_current_user(auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(user_info)))
}

/// GET /api/health — 健康检查
pub async fn health() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("服务运行正常"))
}
