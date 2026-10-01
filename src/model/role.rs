//! 角色与权限数据模型
//!
//! 对应 RBAC 权限系统的 `roles` 和 `user_roles` 表。

use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 管理员角色标识
pub const ADMIN_ROLE: &str = "admin";

/// 内置角色：**不可删除、也不可改名**
///
/// `RbacService::init_defaults` 只在 `roles` 表为空时才写入这两个角色。
/// 一旦删掉且表中仍有其他角色，种子不会重建它们——系统将**永久**失去该角色，
/// 再也无法把任何用户设为管理员。改名同样致命：`ADMIN_ROLE = "admin"`
/// 是最后一名管理员保护与权限码种子的查找依据，一旦改名这些依据全部落空。
/// 因此角色写入路径必须拒绝对内置角色的改名，并拒绝把自定义角色改成内置名。
pub const BUILTIN_ROLES: [&str; 2] = [ADMIN_ROLE, "user"];

/// `roles.name` 的数据库列宽（见 `migrations/002_create_rbac.sql`）。
/// 写入前按同一上限校验，避免把超长名字丢给数据库变成 500。
const ROLE_NAME_MAX_LEN: usize = 50;

/// 归一化角色名——**角色名的唯一数据源**
///
/// 角色名不只是展示用：它是 RBAC 里的授权键（权限码按角色匹配、
/// 用户表单按名字提交）。因此 `roles` 表里存的名字必须始终是同一个
/// canonical 形式，否则"下拉里选得到、提交后端查不到"这类错配无从排查。
///
/// 规则：`trim().to_lowercase()` + 非空 + 不超列宽 + 不含控制字符。
///
/// **允许中间有空格**：角色名是 JSON 里的字符串，空格能原样往返（下拉、请求体、
/// 权限码按名精确匹配都不受影响），它不是"看不见的错配"。大小写与首尾空白才是——
/// 它们会被静默改写，曾导致"下拉里选得到、提交后端查不到"，故必须归一化。
/// 不限制为 `[a-z0-9_-]`：中文角色名是合理需求，限制它没有技术收益。
///
/// 所有角色**写入**路径（新建/更新角色、创建/更新用户、追加角色）
/// 与用户表单取值路径都必须过这个函数。
pub fn normalize_role_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim().to_lowercase();
    if name.is_empty() {
        return Err(AppError::BadRequest("角色名不能为空".into()));
    }
    if name.chars().count() > ROLE_NAME_MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "角色名长度不能超过 {ROLE_NAME_MAX_LEN} 个字符"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::BadRequest("角色名不能包含控制字符".into()));
    }
    Ok(name)
}

/// 角色表记录实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RoleRow {
    /// 角色唯一标识
    pub id: Uuid,
    /// 角色名称标识（admin / user）
    pub name: String,
    /// 角色描述
    pub description: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

/// 用户-角色关联实体
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserRole {
    /// 用户 ID
    pub user_id: Uuid,
    /// 角色 ID
    pub role_id: Uuid,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_lowercases() {
        assert_eq!(normalize_role_name("  Admin  ").unwrap(), "admin");
        assert_eq!(normalize_role_name("Auditor").unwrap(), "auditor");
        assert_eq!(normalize_role_name("AUDIT").unwrap(), "audit");
    }

    #[test]
    fn keeps_canonical_name_unchanged() {
        assert_eq!(normalize_role_name("auditor").unwrap(), "auditor");
        assert_eq!(normalize_role_name(ADMIN_ROLE).unwrap(), ADMIN_ROLE);
    }

    #[test]
    fn rejects_empty() {
        assert!(normalize_role_name("").is_err());
        assert!(normalize_role_name("   ").is_err());
    }

    #[test]
    fn rejects_names_longer_than_the_column() {
        assert!(normalize_role_name(&"a".repeat(50)).is_ok());
        assert!(normalize_role_name(&"a".repeat(51)).is_err());
    }

    /// 列宽按字符计（Postgres VARCHAR(50) 不按字节），
    /// 因此 50 个汉字必须放行——用字节长度会把它误判成超长
    #[test]
    fn measures_length_in_characters_not_bytes() {
        assert!(normalize_role_name(&"审".repeat(50)).is_ok());
        assert!(normalize_role_name(&"审".repeat(51)).is_err());
    }

    #[test]
    fn rejects_control_chars() {
        assert!(normalize_role_name("tab\there").is_err());
        assert!(normalize_role_name("new\nline").is_err());
    }

    /// 中间有空格是合法的：它能原样往返，不会造成"选得到却存不进去"的静默错配。
    /// 大小写与首尾空白才是——那两种会被静默改写，必须归一化。
    #[test]
    fn allows_inner_whitespace() {
        assert_eq!(normalize_role_name("two words").unwrap(), "two words");
        assert_eq!(
            normalize_role_name("  Senior Auditor  ").unwrap(),
            "senior auditor"
        );
    }

    #[test]
    fn builtin_roles_survive_normalization() {
        for name in BUILTIN_ROLES {
            assert_eq!(normalize_role_name(name).unwrap(), name);
        }
    }
}
