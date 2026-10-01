//! 权限码提取器
//!
//! 每个权限码对应一个**类型化提取器**（如 `PermUserCreate`），在提取阶段完成校验。
//!
//! ## 为什么不直接在 handler 里写 `perm.require(CODE)?`
//!
//! axum 按参数顺序执行提取器，`Json<T>` 是最后一个（`FromRequest`）。
//! 若把校验写进函数体，`Json` 会先解析请求体：缺字段时返回 422 而非 403，
//! 相当于让无权限调用者拿到接口的参数结构反馈。
//! **鉴权必须早于入参校验**，所以校验放进提取器本身。
//!
//! ## 用法
//!
//! ```ignore
//! pub async fn create_user(
//!     State(state): State<AppState>,
//!     _perm: PermUserCreate,
//!     Json(req): Json<UserManageRequest>,
//! ) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
//!     // 走到这里说明已持有 system:user:create
//! }
//! ```
//!
//! 参数名前缀 `_` 是必须的：提取器只做校验，本身无需在函数体里使用。

use std::collections::HashSet;
use std::marker::PhantomData;

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::model::permission;
use crate::router::AppState;

/// 统一构造 403 响应
fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let body = axum::Json(json!({
        "code": status.as_u16(),
        "message": message.into(),
        "data": null,
    }));
    (status, body).into_response()
}

/// 权限码校验器
///
/// 持有当前用户经 `role_menus` 授权的全部权限码，判定逻辑集中在这里。
pub struct PermissionGuard {
    codes: HashSet<String>,
    /// 所属用户名，仅用于日志定位
    username: String,
}

impl PermissionGuard {
    /// 该用户是否拥有指定权限码
    pub fn has(&self, code: &str) -> bool {
        self.codes.contains(code)
    }

    /// 该用户拥有的全部权限码（字典序）
    pub fn codes(&self) -> Vec<&str> {
        let mut codes: Vec<&str> = self.codes.iter().map(String::as_str).collect();
        codes.sort_unstable();
        codes
    }

    /// 要求指定权限码，否则返回 403
    fn require(&self, code: &str) -> Result<(), AppError> {
        if self.has(code) {
            return Ok(());
        }
        tracing::warn!(
            "权限码不足: 用户 {} 需要 {code}，现有 {:?}",
            self.username,
            self.codes()
        );
        Err(AppError::PermissionDenied(code.to_string()))
    }

    /// 从请求扩展取认证用户，按其角色解析权限码
    async fn load(parts: &Parts, state: &AppState) -> Result<Self, AppError> {
        let auth_user = parts
            .extensions
            .get::<AuthenticatedUser>()
            .ok_or(AppError::Unauthorized)?;

        let codes = state
            .menu_repo
            .find_permission_codes(&auth_user.roles)
            .await?;
        tracing::debug!(
            "用户 {} 解析到 {} 个权限码",
            auth_user.username,
            codes.len()
        );

        Ok(PermissionGuard {
            codes: codes.into_iter().collect(),
            username: auth_user.username.clone(),
        })
    }
}

/// 为每个权限码生成一个类型化提取器
///
/// 生成的类型在提取阶段校验权限码，因此：
/// - 早于 `Json` / `Path` 等入参提取，鉴权先于校验
/// - 权限码与 handler 在编译期绑定，不会写错也删不掉
macro_rules! permission_guards {
    ($($ty:ident => $code:expr),* $(,)?) => {
        $(
            #[doc = concat!(
                "提取阶段校验权限码 `", stringify!($code),
                "`（早于入参解析，鉴权先于校验）"
            )]
            pub struct $ty(PhantomData<()>);

            impl FromRequestParts<AppState> for $ty {
                type Rejection = Response;

                async fn from_request_parts(
                    parts: &mut Parts,
                    state: &AppState,
                ) -> Result<Self, Self::Rejection> {
                    let guard = match PermissionGuard::load(parts, state).await {
                        Ok(guard) => guard,
                        Err(AppError::Unauthorized) => {
                            return Err(error_response(StatusCode::UNAUTHORIZED, "未认证，请先登录"))
                        }
                        Err(e) => return Err(e.into_response()),
                    };

                    match guard.require($code) {
                        Ok(()) => Ok($ty(PhantomData)),
                        Err(AppError::PermissionDenied(_)) => {
                            Err(error_response(StatusCode::FORBIDDEN, format!("缺少权限：{}", $code)))
                        }
                        Err(e) => Err(e.into_response()),
                    }
                }
            }
        )*
    };
}

permission_guards! {
    // 用户管理
    PermUserList => permission::USER_LIST,
    PermUserCreate => permission::USER_CREATE,
    PermUserUpdate => permission::USER_UPDATE,
    PermUserDelete => permission::USER_DELETE,
    // 角色管理
    PermRoleList => permission::ROLE_LIST,
    PermRoleCreate => permission::ROLE_CREATE,
    PermRoleUpdate => permission::ROLE_UPDATE,
    PermRoleDelete => permission::ROLE_DELETE,
    // 菜单管理
    PermMenuList => permission::MENU_LIST,
    PermMenuCreate => permission::MENU_CREATE,
    PermMenuUpdate => permission::MENU_UPDATE,
    PermMenuDelete => permission::MENU_DELETE,
    PermMenuGrant => permission::MENU_GRANT,
    // 字典管理
    PermDictList => permission::DICT_LIST,
    PermDictCreate => permission::DICT_CREATE,
    PermDictUpdate => permission::DICT_UPDATE,
    PermDictDelete => permission::DICT_DELETE,
    PermDictRefresh => permission::DICT_REFRESH,
    // 审计日志
    PermLogList => permission::LOG_LIST,
    PermLogExport => permission::LOG_EXPORT,
    // 系统监控
    PermMonitorSystem => permission::MONITOR_SYSTEM,
    PermMonitorApi => permission::MONITOR_API,
    PermMonitorAlert => permission::MONITOR_ALERT,
    PermMonitorReset => permission::MONITOR_RESET,
    PermMonitorExport => permission::MONITOR_EXPORT,
    // 其他
    PermExportUser => permission::EXPORT_USER,
    PermTestAccess => permission::TEST_ACCESS,
    PermValidateTest => permission::VALIDATE_TEST,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(codes: &[&str]) -> PermissionGuard {
        PermissionGuard {
            codes: codes.iter().map(|c| c.to_string()).collect(),
            username: "u".to_string(),
        }
    }

    #[test]
    fn has_reflects_granted_codes_only() {
        let g = guard(&[permission::USER_LIST, permission::USER_CREATE]);
        assert!(g.has(permission::USER_LIST));
        assert!(!g.has(permission::USER_DELETE));
    }

    #[test]
    fn codes_are_sorted_for_stable_logs() {
        let g = guard(&[permission::USER_LIST, permission::USER_CREATE]);
        assert_eq!(
            g.codes(),
            vec![permission::USER_CREATE, permission::USER_LIST]
        );
    }

    #[test]
    fn require_passes_on_granted_code() {
        assert!(guard(&[permission::USER_DELETE])
            .require(permission::USER_DELETE)
            .is_ok());
    }

    #[test]
    fn require_names_the_missing_code() {
        let err = guard(&[])
            .require(permission::USER_DELETE)
            .expect_err("空权限集合应拒绝");
        match err {
            AppError::PermissionDenied(code) => assert_eq!(code, permission::USER_DELETE),
            other => panic!("期望 PermissionDenied，实际 {other:?}"),
        }
    }
}
