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

/// 把"显式传了 `null`"与"值本身"区分开的反序列化器
///
/// serde 对 `Option<T>` 的默认处理是：**字段缺失**和**值为 `null`**都得到 `None`。
/// 对多数可选字段这没问题（两种情况都表示"不改"），但 `parent_id` 不一样——
/// 它有第三种合法取值 `NULL` 表示"把菜单摘成根"。
///
/// 于是必须用双层 `Option` 表达三态：
/// - `None` → 本次请求没提父级，保持原样
/// - `Some(None)` → 显式摘成根
/// - `Some(Some(id))` → 挂到 `id` 下
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    // 值本身是 `None`（JSON null）时包成 `Some(None)`；
    // 有值时得到 `Some(Some(v))`。字段整个缺失则由 `#[serde(default)]` 兜成 `None`。
    Deserialize::deserialize(deserializer).map(Some)
}

/// 更新菜单请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateMenuRequest {
    /// 上级菜单，**三态**（见 [`double_option`]）
    ///
    /// 曾经这里是 `Option<Uuid>`，配合 `fields.parent_id.or(menu.parent_id)`
    /// 的写法，"把菜单摘成根"会被当成"本次不改父级"：
    /// 请求返回 200、字段原样回显，而结构纹丝不动，没有任何提示说它被忽略了。
    /// 前端又没有父级字段，于是管理员**没有任何途径调整菜单层级**。
    #[serde(default, deserialize_with = "double_option")]
    pub parent_id: Option<Option<Uuid>>,
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

/// 走不到根、因而**不在任何菜单树里**的菜单（`GET /api/admin/menus/diagnostics`）
///
/// 单列一个类型而不是复用 `MenuNode`：这些节点没有可用层级，
/// 而 `MenuNode.children` 恰恰是这里给不出的东西——塞一个空 `children`
/// 会让界面以为它是个正常的叶子菜单。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UnreachableMenu {
    pub id: Uuid,
    pub name: String,
    /// 当前记录的父级；`None` 表示它自己就是根（因而一定可达，不该出现在这里）
    pub parent_id: Option<Uuid>,
    /// 成环还是悬空引用——两者修复动作相同，但排查方向不同，值得说清
    pub reason: String,
    pub sort_order: i32,
}

#[cfg(test)]
mod tests {
    use super::UpdateMenuRequest;

    fn parse(json: &str) -> UpdateMenuRequest {
        serde_json::from_str(json).expect("请求体应可解析")
    }

    /// `parent_id` 必须区分**三**种情况，少一种就会退回"摘成根不可用"的老问题
    #[test]
    fn parent_id_distinguishes_absent_null_and_a_value() {
        let absent = parse(r#"{"name":"x"}"#).parent_id;
        assert!(
            absent.is_none(),
            "字段缺失应得到 None（本次不改父级），实际: {absent:?}"
        );

        let explicit_null = parse(r#"{"name":"x","parent_id":null}"#).parent_id;
        assert!(
            matches!(explicit_null, Some(None)),
            "显式 null 应得到 Some(None)（摘成根），实际: {explicit_null:?}"
        );

        let id = uuid::Uuid::new_v4();
        let with_value = parse(&format!(r#"{{"name":"x","parent_id":"{id}"}}"#)).parent_id;
        assert!(
            matches!(with_value, Some(Some(v)) if v == id),
            "给了值应得到 Some(Some(id))，实际: {with_value:?}"
        );
    }
}
