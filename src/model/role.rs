//! 角色与权限数据模型
//!
//! 对应 RBAC 权限系统的 `roles` 和 `user_roles` 表。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 管理员角色标识
pub const ADMIN_ROLE: &str = "admin";

/// 内置角色：允许分配给用户，但**不可删除**
///
/// `RbacService::init_defaults` 只在 `roles` 表为空时才写入这两个角色。
/// 一旦删掉且表中仍有其他角色，种子不会重建它们——系统将**永久**失去该角色，
/// 再也无法把任何用户设为管理员。因此删除接口必须拒绝内置角色。
pub const BUILTIN_ROLES: [&str; 2] = [ADMIN_ROLE, "user"];

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
