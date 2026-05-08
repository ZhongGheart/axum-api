//! JWT 鉴权中间件
//!
//! 从请求的 `Authorization: Bearer <token>` 头中提取并验证 JWT 令牌。
//! 验证通过后将用户信息注入到请求扩展中，供后续处理器使用。
//!
//! 同时提供 `require_role` 函数用于接口权限拦截。

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::router::AppState;
use crate::utils::jwt::Claims;

/// 认证用户信息，通过中间件注入到请求扩展中
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    /// 用户 ID
    pub user_id: Uuid,
    /// 用户角色
    pub role: String,
}

/// JWT 鉴权中间件
///
/// 从请求头提取 Bearer Token，使用 AppState 中的 JwtUtil 验证其有效性。
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    // 获取 Authorization 头
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            let body = Json(json!({
                "code": 401,
                "message": "缺少 Authorization 请求头",
                "data": null,
            }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        })?;

    // 解析 Bearer Token
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| {
            let body = Json(json!({
                "code": 401,
                "message": "Authorization 格式错误，请使用 Bearer <token>",
                "data": null,
            }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        })?;

    // 使用 AppState 中的 JwtUtil 验证令牌
    let claims: Claims = state.jwt_util.verify(token).map_err(|_| {
        let body = Json(json!({
            "code": 401,
            "message": "令牌无效或已过期",
            "data": null,
        }));
        (StatusCode::UNAUTHORIZED, body).into_response()
    })?;

    // 将认证用户信息注入请求扩展
    let authenticated_user = AuthenticatedUser {
        user_id: claims.sub,
        role: claims.role,
    };
    req.extensions_mut().insert(authenticated_user);

    Ok(next.run(req).await)
}

// ============================================
// Extractor：在处理器中直接提取认证用户
// ============================================

/// 从请求中提取认证用户信息的 Axum Extractor
///
/// 在需要认证的路由处理器中，直接声明参数 `auth_user: AuthenticatedUser`
/// 即可获取当前登录用户信息。
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AuthenticatedUser>().cloned().ok_or_else(|| {
            let body = Json(json!({
                "code": 401,
                "message": "未认证，请先登录",
                "data": null,
            }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        })
    }
}

/// 角色权限验证中间件
///
/// 用于需要特定角色才能访问的路由。
///
/// # 示例
///
/// ```ignore
/// .route_layer(middleware::from_fn(require_role("admin")))
/// ```
#[allow(dead_code)]
pub async fn require_role(
    role: &'static str,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let auth_user = req
        .extensions()
        .get::<AuthenticatedUser>()
        .ok_or_else(|| {
            let body = Json(json!({
                "code": 401,
                "message": "未认证",
                "data": null,
            }));
            (StatusCode::UNAUTHORIZED, body).into_response()
        })?;

    if auth_user.role != role {
        let body = Json(json!({
            "code": 403,
            "message": format!("需要 {} 角色权限，当前角色: {}", role, auth_user.role),
            "data": null,
        }));
        return Err((StatusCode::FORBIDDEN, body).into_response());
    }

    Ok(next.run(req).await)
}
