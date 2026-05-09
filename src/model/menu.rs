//! 菜单数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 菜单数据库实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Menu {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub path: Option<String>,
    pub component: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub r#type: String,
    pub permission: Option<String>,
    pub is_visible: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 菜单树节点（含子节点）
#[derive(Debug, Serialize, Deserialize)]
pub struct MenuNode {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub path: Option<String>,
    pub component: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub r#type: String,
    pub permission: Option<String>,
    pub is_visible: bool,
    pub created_at: DateTime<Utc>,
    pub children: Vec<MenuNode>,
}

impl From<Menu> for MenuNode {
    fn from(m: Menu) -> Self {
        Self {
            id: m.id,
            parent_id: m.parent_id,
            name: m.name,
            path: m.path,
            component: m.component,
            icon: m.icon,
            sort_order: m.sort_order,
            r#type: m.r#type,
            permission: m.permission,
            is_visible: m.is_visible,
            created_at: m.created_at,
            children: vec![],
        }
    }
}

/// 创建菜单请求
#[derive(Debug, Deserialize)]
pub struct CreateMenuRequest {
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub path: Option<String>,
    pub component: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub r#type: String,
    pub permission: Option<String>,
    pub is_visible: Option<bool>,
}

/// 更新菜单请求
#[derive(Debug, Deserialize)]
pub struct UpdateMenuRequest {
    pub parent_id: Option<Uuid>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub component: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub r#type: Option<String>,
    pub permission: Option<String>,
    pub is_visible: Option<bool>,
}

/// 分配菜单权限请求
#[derive(Debug, Deserialize)]
pub struct AssignMenuRequest {
    pub menu_ids: Vec<Uuid>,
}
