//! 角色与权限数据模型
//!
//! 对应 RBAC 权限系统的 `roles` 和 `user_roles` 表。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
