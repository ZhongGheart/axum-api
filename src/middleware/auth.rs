//! JWT 鉴权中间件
//!
//! 从请求的 `Authorization: Bearer <token>` 头中提取并验证 JWT 令牌。
//! 验证通过后将用户信息注入到请求扩展中，供后续处理器使用。
//!
//! 粗粒度的角色闸门已于 v0.5.0 PR-3 移除：`require_role` 连同 `router` 里 5 处
//! `require_role("admin")` 一起删掉了。
//!
//! 两者曾是 AND 语义——角色闸门天然是权限码的上游闸门，因此也顺带挡住了
//! "持有 `system:user:create` 就能建出 admin 用户"这类提权。闸门一撤，
//! 授权改由权限码独占决定，**能授予的 ⊆ 自己已持有的**这层下界
//! 见 [`crate::middleware::permission`]。保留一个无人调用的角色闸门
//! 只会在下一次改动里被误当成"更安全的兜底"接回去，所以直接删掉。
//!
//! 权限码存放在 `menus.permission`（`type='button'` 的菜单行），经 `role_menus` 授权，
//! 定义见 [`crate::model::permission`]。

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

/// 快速构造 JSON 错误响应
fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let body = Json(json!({
        "code": status.as_u16(),
        "message": message.into(),
        "data": null,
    }));
    (status, body).into_response()
}

/// 认证用户信息，通过中间件注入到请求扩展中
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    /// 用户 ID
    pub user_id: Uuid,
    /// 用户主要角色
    pub role: String,
    /// 用户拥有的所有角色标识列表
    pub roles: Vec<String>,
    /// 用户名（审计日志展示用）
    pub username: String,
    /// 当前令牌的 jti（用于单令牌注销）
    pub token_jti: String,
    /// JWT 过期时间戳（用于黑名单 TTL）
    pub token_exp: u64,
}

/// JWT 鉴权中间件
///
/// 1. 从请求头提取 Bearer Token
/// 2. 验证 JWT 签名与有效期
/// 3. 检查 Token 是否在 Redis 黑名单中（已下线）
/// 4. 将用户信息注入请求扩展
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("auth_middleware: 缺少 Authorization 请求头");
            error_response(StatusCode::UNAUTHORIZED, "缺少 Authorization 请求头")
        })?;

    let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        tracing::warn!(
            "auth_middleware: Authorization 格式错误: {}",
            &auth_header[..20.min(auth_header.len())]
        );
        error_response(
            StatusCode::UNAUTHORIZED,
            "Authorization 格式错误，请使用 Bearer <token>",
        )
    })?;

    let claims: Claims = state.jwt_util.verify(token).map_err(|e| {
        tracing::warn!("auth_middleware: JWT 验证失败: {:?}", e);
        error_response(StatusCode::UNAUTHORIZED, "令牌无效或已过期")
    })?;

    // 单令牌注销校验（登出/下线）：Redis 不可用时 fail-closed
    let blacklisted = state
        .redis_client
        .is_token_blacklisted(&claims.jti)
        .await
        .map_err(|e| {
            tracing::error!("auth_middleware: 令牌黑名单校验失败: {e}");
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "认证依赖不可用，请稍后重试",
            )
        })?;
    if blacklisted {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "令牌已被注销，请重新登录",
        ));
    }

    // 全量会话吊销校验（改密/停用/删除账号）
    let revoked_before = state
        .redis_client
        .user_revoked_before(&claims.sub)
        .await
        .map_err(|e| {
            tracing::error!("auth_middleware: 会话吊销校验失败: {e}");
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "认证依赖不可用，请稍后重试",
            )
        })?;
    // 用毫秒级 `iat_ms` 比对，不用秒级 `iat`：秒级精度分不清
    // "同一秒内先签发还是后签发"，会导致要么放过旧令牌、要么误伤新登录。
    // 旧令牌（升级前签发）没有 `iat_ms`，serde 默认为 0，
    // 一定小于任何吊销水位 —— 方向是失效，安全的一侧。
    if matches!(revoked_before, Some(ts) if claims.iat_ms < ts) {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "登录状态已失效，请重新登录",
        ));
    }

    let authenticated_user = AuthenticatedUser {
        user_id: claims.sub,
        username: claims.username.clone(),
        role: claims.role.clone(),
        roles: claims.roles.clone(),
        token_jti: claims.jti.clone(),
        token_exp: claims.exp,
    };
    req.extensions_mut().insert(authenticated_user);

    Ok(next.run(req).await)
}

// ============================================
// Extractor：在处理器中直接提取认证用户
// ============================================

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "未认证，请先登录"))
    }
}
