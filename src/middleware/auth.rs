//! JWT 鉴权中间件
//!
//! 从请求的 `Authorization: Bearer <token>` 头中提取并验证 JWT 令牌。
//! 验证通过后将用户信息注入到请求扩展中，供后续处理器使用。
//!
//! 同时提供 `require_role`（粗粒度角色闸门）与 [`PermissionGuard`]（细粒度权限码闸门）。
//!
//! 两者是 AND 语义：`require_role("admin")` 决定"能不能进管理区"，
//! `PermissionGuard::require(权限码)` 决定"能进管理区的哪一部分"。
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

use crate::error::AppError;
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
    if matches!(revoked_before, Some(ts) if claims.iat < ts) {
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

/// 角色权限验证中间件
pub async fn require_role(
    role: &'static str,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let auth_user = req
        .extensions()
        .get::<AuthenticatedUser>()
        .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "未认证"))?;

    let has_role = auth_user.roles.iter().any(|r| r == role) || auth_user.role == role;

    if !has_role {
        tracing::warn!(
            "require_role({}): 用户 {} 角色 {:?} 权限不足",
            role,
            auth_user.user_id,
            auth_user.roles
        );
        return Err(error_response(
            StatusCode::FORBIDDEN,
            format!("需要 {role} 角色权限"),
        ));
    }

    Ok(next.run(req).await)
}

// ============================================
// 权限码：细粒度授权
// ============================================

/// 权限码校验器
///
/// 作为提取器注入 handler，一次查询解析出当前用户的全部权限码，
/// 随后由 handler 显式声明每个动作需要哪个码：
///
/// ```ignore
/// pub async fn delete_user(
///     State(state): State<AppState>,
///     perm: PermissionGuard,
///     Path(id): Path<Uuid>,
/// ) -> Result<Json<ApiResponse<&'static str>>, AppError> {
///     perm.require(permission::USER_DELETE)?;
///     // ...
/// }
/// ```
///
/// 权限码来自数据库实时查询，撤销 `role_menus` 授权后立即失效。
pub struct PermissionGuard {
    /// 当前用户拥有的权限码
    codes: std::collections::HashSet<String>,
}

impl PermissionGuard {
    /// 该用户是否拥有指定权限码
    pub fn has(&self, code: &str) -> bool {
        self.codes.contains(code)
    }

    /// 该用户拥有的全部权限码（按字典序）
    pub fn codes(&self) -> Vec<&str> {
        let mut codes: Vec<&str> = self.codes.iter().map(String::as_str).collect();
        codes.sort_unstable();
        codes
    }

    /// 要求用户拥有指定权限码，否则返回 403
    pub fn require(&self, code: &str) -> Result<(), AppError> {
        if self.has(code) {
            return Ok(());
        }
        tracing::warn!(
            "权限码不足: 需要 {}，用户现有权限码 {:?}",
            code,
            self.codes(),
        );
        Err(AppError::PermissionDenied(code.to_string()))
    }
}

/// 从请求扩展取认证用户，再按其角色解析权限码
///
/// 需要 `AppState` 以访问数据库；所有 handler 最终都经 `Router::with_state(AppState)`，
/// 因此此处具体实现 `FromRequestParts<AppState>`。
impl FromRequestParts<AppState> for PermissionGuard {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_user = parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| error_response(StatusCode::UNAUTHORIZED, "未认证，请先登录"))?;

        let codes = state
            .menu_repo
            .find_permission_codes(&auth_user.roles)
            .await
            .map_err(|e| {
                tracing::error!("解析用户权限码失败: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "权限查询失败")
            })?;

        tracing::debug!(
            "用户 {} 解析到 {} 个权限码",
            auth_user.username,
            codes.len()
        );

        Ok(PermissionGuard {
            codes: codes.into_iter().collect(),
        })
    }
}
