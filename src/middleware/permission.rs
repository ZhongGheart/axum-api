//! 权限码提取器
//!
//! 每个权限码对应一个**类型化提取器**（如 `PermUserCreate`），在提取阶段完成校验。
//!
//! ## 为什么不直接在 handler 里写 `perm.require(CODE)?`
//!
//! axum 按参数顺序执行提取器，`Json<T>` 是最后一个（`FromRequest`）。
//! 若把校验写进函数体，`Json` 会先解析请求体：缺字段时返回 400 而非 403，
//! 相当于让无权限调用者拿到接口的参数结构反馈。
//! **鉴权必须早于入参校验**，所以校验放进提取器本身。
//!
//! ## 用法
//!
//! ```ignore
//! pub async fn create_user(
//!     State(state): State<AppState>,
//!     perm: PermUserCreate,
//!     Json(req): Json<UserManageRequest>,
//! ) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
//!     // 走到这里说明已持有 system:user:create
//! }
//! ```
//!
//! 参数名前缀 `_` 不是必须的：只做校验、不在函数体里用时写 `_perm` 即可；
//! 需要再做"能否授予他人权限"这类判定时，去掉 `_` 取 `perm.guard()`。
//!
//! ## 授权下界：能授予的 ⊆ 已持有的
//!
//! 权限码只回答"这个接口能不能调"。**它不回答"能不能把权限给别人"**。
//! 撤掉 `require_role("admin")` 之后必须自己补上这一层，否则持有
//! `system:user:create` 的角色能直接建出 admin 用户。
//!
//! [`PermissionGuard::ensure_covers`] 就是这层下界，判定依据是权限码包含关系
//! 而非角色名——与"角色是数据、不是常量"一致。

use std::collections::HashSet;

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

    /// 该用户是否覆盖 `required` 中的**每一个**权限码
    ///
    /// 这是 v0.5.0 PR-3 撤掉 `require_role("admin")` 之后的授权下界：
    /// **你能授予的权限，必须全部是你自己已持有的。**
    ///
    /// 此前 `require_role("admin")` 与权限码守卫是 AND 语义，角色闸门天然盖住了
    /// 提权路径。闸门一撤，持有 `system:user:create` 的自定义角色就能建出 admin
    /// 用户——**创建用户即等于授予管理员**。用包含关系判定后，这条约束与
    /// "admin" 这个名字无关：谁持有全部权限码，谁才有能力授予全部权限码。
    ///
    /// 返回**第一个**未覆盖的权限码，供 403 文案指名道姓。
    pub fn first_uncovered<'a>(&self, required: &'a [String]) -> Option<&'a str> {
        required
            .iter()
            .find(|code| !self.codes.contains(code.as_str()))
            .map(String::as_str)
    }

    /// 断言 `required` 全被覆盖，否则 `PermissionDenied`
    ///
    /// 提示语点名 `action` 与缺失的码：单说"权限不足"，管理员在授权树里
    /// 看不出该去勾哪个按钮，也看不出是哪一步被拒。
    pub fn ensure_covers(&self, required: &[String], action: &str) -> Result<(), AppError> {
        match self.first_uncovered(required) {
            None => Ok(()),
            Some(missing) => Err(AppError::PermissionDenied(format!(
                "{action}需要「{missing}」，而你未持有该权限码；只能授予自己已持有的权限"
            ))),
        }
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

// ────────────────────────────────────────────
// 授权下界：把"目标对象"翻译成权限码，再与调用者已持有的码比较
// ────────────────────────────────────────────

/// 一组角色经 `role_menus` 授权后实际携带的权限码
pub async fn codes_of_roles(state: &AppState, roles: &[String]) -> Result<Vec<String>, AppError> {
    state.menu_repo.find_permission_codes(roles).await
}

/// 断言调用者有权把 `target_roles` 授予他人（建用户 / 改用户 / 追加角色）
///
/// 判定依据是权限码包含关系，**不是角色名**——与"角色是数据不是常量"一致。
/// 副作用是它天然堵住三条提权路径：建出 admin 用户、追加 admin 角色、
/// 以及把权限比自己高的角色指派给自己。
///
/// `target_roles` 是**目标对象持有的角色**：授予角色时传将被赋予的角色，
/// 改动/删除/停用/重置密码某用户时传该用户当前的角色
/// （**重置一个权限高于自己的账号的密码，等于直接登录成那个账号**，
/// 比授予角色更直接，所以这几条写路径不能只看 `system:user:update`）。
pub async fn ensure_can_grant_roles(
    state: &AppState,
    guard: &PermissionGuard,
    target_roles: &[String],
    action: &str,
) -> Result<(), AppError> {
    let required = codes_of_roles(state, target_roles).await?;
    guard.ensure_covers(&required, action)
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
            pub struct $ty(PermissionGuard);

            impl $ty {
                /// 取出底层权限守卫
                ///
                /// handler 多数时候不需要它（提取阶段已经校验过），
                /// 但涉及"能否授予他人权限"时要用 `guard().ensure_covers(..)`。
                pub fn guard(&self) -> &PermissionGuard {
                    &self.0
                }
            }

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
                        Ok(()) => Ok($ty(guard)),
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

    // ── 授权下界：能授予的必须是自己已持有的 ──────────────────

    fn owned(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn covering_a_superset_is_allowed() {
        let g = guard(&[permission::USER_LIST, permission::USER_CREATE]);
        assert_eq!(g.first_uncovered(&owned(&[permission::USER_CREATE])), None);
        assert!(g
            .ensure_covers(&owned(&[permission::USER_LIST]), "授予角色")
            .is_ok());
    }

    #[test]
    fn empty_target_is_always_coverable() {
        // 无权限码的角色（如 `user`）任何人都能授予——它不授予任何能力
        assert_eq!(guard(&[]).first_uncovered(&[]), None);
        assert!(guard(&[]).ensure_covers(&[], "授予角色").is_ok());
    }

    #[test]
    fn granting_a_code_you_lack_is_denied() {
        // 核心防线：持有 user:create 的角色不能借它授予自己没有的 user:delete
        let g = guard(&[permission::USER_CREATE]);
        let err = g
            .ensure_covers(
                &owned(&[permission::USER_CREATE, permission::USER_DELETE]),
                "授予角色",
            )
            .expect_err("多出一个未持有的码应被拒绝");
        match err {
            AppError::PermissionDenied(msg) => {
                assert!(
                    msg.contains(permission::USER_DELETE),
                    "应指名缺失的码: {msg}"
                );
                assert!(msg.contains("授予角色"), "应说明是哪一步被拒: {msg}");
            }
            other => panic!("期望 PermissionDenied，实际 {other:?}"),
        }
    }

    #[test]
    fn first_uncovered_reports_the_first_missing_code_in_order() {
        let g = guard(&[permission::USER_LIST]);
        let required = owned(&[
            permission::ROLE_LIST,
            permission::ROLE_CREATE,
            permission::USER_LIST,
        ]);
        assert_eq!(g.first_uncovered(&required), Some(permission::ROLE_LIST));
    }

    #[test]
    fn identical_permission_set_is_not_a_subset_violation() {
        // 两个都持 admin 全码的角色互相授权必须放行（等集是子集）
        let g = guard(&[permission::USER_LIST, permission::USER_DELETE]);
        assert!(g
            .ensure_covers(
                &owned(&[permission::USER_DELETE, permission::USER_LIST]),
                "授予角色"
            )
            .is_ok());
    }
}
