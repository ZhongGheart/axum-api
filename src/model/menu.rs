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
    /// 最近一次被清空掉的权限码（见迁移 `009`）
    ///
    /// 存在的唯一理由是**恢复**：权限码一旦被清空，全系统就没有任何角色
    /// 再持有它，而 `update_menu` 的守卫要求"改写权限码必须持有目标码"，
    /// 于是写回去会被自己的守卫拦死。这里留住旧值，
    /// 让清空者能撤销自己的误操作。
    #[serde(default)]
    pub prev_permission: Option<String>,
    /// 清空 `prev_permission` 的操作者 id
    ///
    /// 恢复接口只放行"本人撤销本人的误操作"——能清空已授予角色的按钮
    /// 说明当时就持有该码，恢复即回到清空前状态，净零提权。
    /// 存 id 而非用户名：用户名可改，id 不会。
    #[serde(default)]
    pub prev_permission_cleared_by: Option<Uuid>,
}

/// 菜单树节点（含子节点）
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(no_recursion)]
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
    /// 可恢复的权限码；`None` 表示没有可恢复的清空记录
    ///
    /// 刻意**不**把 `prev_permission_cleared_by` 暴露给前端：
    /// 界面只需要知道"能不能恢复"，"是不是你清的"由服务端判定，
    /// 客户端不该也不能靠它做鉴权。
    pub restorable_permission: Option<String>,
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
            restorable_permission: m.prev_permission,
            children: vec![],
        }
    }
}

/// 创建菜单请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AssignMenuRequest {
    pub menu_ids: Vec<Uuid>,
}
