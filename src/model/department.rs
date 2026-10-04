//! 部门 / 组织树数据模型
//!
//! 对应数据库中 `departments` 表的实体结构。
//!
//! 树形结构用自引用表（`parent_id` 指向自己）表达，
//! 查询时用应用层 [`build_tree`] 构建树（与菜单树同一模式）。
//!
//! ── 为什么不用递归 CTE ──────────────────────────────────────
//! 菜单树已经用应用层 `build_tree`，部门树复用同一模式：
//! - 部门数量级在几十到几百，应用层构建完全够用
//! - 两处用同一套树构建逻辑，"什么算根"的定义不会分叉
//! - 递归 CTE 在环上会死循环，应用层 `build_tree` 会把环上无一为根的
//!   整支剪掉（见 `build_tree` 的文档）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// 部门数据库实体
///
/// 映射 `departments` 表的每一行记录。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Department {
    /// 部门唯一标识
    pub id: Uuid,
    /// 父部门 ID；`None` 表示根部门
    pub parent_id: Option<Uuid>,
    /// 部门名称
    pub name: String,
    /// 部门描述
    pub description: Option<String>,
    /// 同级排序权重（升序）
    pub sort_order: i32,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
}

/// 部门树节点
///
/// 在 [`Department`] 的基础上加 `children`，用于树形返回。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(no_recursion)]
pub struct DepartmentNode {
    /// 部门唯一标识
    pub id: Uuid,
    /// 父部门 ID；`None` 表示根部门
    pub parent_id: Option<Uuid>,
    /// 部门名称
    pub name: String,
    /// 部门描述
    pub description: Option<String>,
    /// 同级排序权重
    pub sort_order: i32,
    /// 直接子部门（按 `sort_order` 升序）
    pub children: Vec<DepartmentNode>,
}

impl From<Department> for DepartmentNode {
    fn from(d: Department) -> Self {
        Self {
            id: d.id,
            parent_id: d.parent_id,
            name: d.name,
            description: d.description,
            sort_order: d.sort_order,
            children: Vec::new(),
        }
    }
}

/// 扁平列表项（用于下拉选择）
///
/// 与 [`DepartmentNode`] 分开是因为下拉选择不需要 `children`，
/// 而树形返回不需要 `level` / `path`。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DepartmentFlat {
    /// 部门唯一标识
    pub id: Uuid,
    /// 父部门 ID；`None` 表示根部门
    pub parent_id: Option<Uuid>,
    /// 部门名称
    pub name: String,
    /// 层级深度（根部门为 0）
    pub level: i32,
    /// 从根到当前节点的路径（如 `总公司/技术部/后端组`）
    pub path: String,
}

/// 部门下的用户
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct DepartmentUser {
    /// 用户唯一标识
    pub id: Uuid,
    /// 用户名
    pub username: String,
    /// 展示名
    pub display_name: Option<String>,
    /// 电子邮箱
    pub email: String,
    /// 是否激活
    pub is_active: bool,
}

/// 新建部门请求
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateDepartmentRequest {
    /// 父部门 ID；`None` 表示根部门
    pub parent_id: Option<Uuid>,
    /// 部门名称（1-100 字符）
    pub name: String,
    /// 部门描述
    pub description: Option<String>,
    /// 同级排序权重
    pub sort_order: Option<i32>,
}

/// 修改部门请求
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateDepartmentRequest {
    /// 部门名称
    pub name: Option<String>,
    /// 部门描述（`null` 表示清空）
    pub description: Option<Option<String>>,
    /// 同级排序权重
    pub sort_order: Option<i32>,
}

/// 移动部门请求
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MoveDepartmentRequest {
    /// 新的父部门 ID；`None` 表示移到根
    pub new_parent_id: Option<Uuid>,
}

/// 从扁平列表构建树
///
/// 与菜单树的 `build_tree` 同一模式：
/// 1. 判根看"父节点是否在集合内"——不在集合内就当根
/// 2. 递归收集直接子节点
///
/// **环的处理**：如果数据里存在环（A 的 parent 是 B，B 的 parent 是 A），
/// 环上无一为根，整支会被静默剪掉。这是**可接受**的：
/// 环是数据错误，服务层在写入时会防住（见 `DepartmentService::move`），
/// 查询时剪掉比死循环好。
pub fn build_tree(all: &[Department]) -> Vec<DepartmentNode> {
    let ids: std::collections::HashSet<Uuid> = all.iter().map(|d| d.id).collect();
    let is_root = |d: &Department| match d.parent_id {
        Some(pid) => !ids.contains(&pid),
        None => true,
    };

    let mut roots: Vec<DepartmentNode> = all
        .iter()
        .filter(|d| is_root(d))
        .map(|d| DepartmentNode::from(d.clone()))
        .collect();

    for root in &mut roots {
        build_children(all, root);
    }

    // 根部门按 sort_order 升序
    roots.sort_by_key(|d| d.sort_order);
    roots
}

/// 递归收集 `parent` 的直接子节点
fn build_children(all: &[Department], parent: &mut DepartmentNode) {
    let mut children: Vec<DepartmentNode> = all
        .iter()
        .filter(|d| d.parent_id == Some(parent.id))
        .map(|d| DepartmentNode::from(d.clone()))
        .collect();

    for child in &mut children {
        build_children(all, child);
    }

    children.sort_by_key(|d| d.sort_order);
    parent.children = children;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dept(id: Uuid, parent: Option<Uuid>, name: &str, sort: i32) -> Department {
        Department {
            id,
            parent_id: parent,
            name: name.to_string(),
            description: None,
            sort_order: sort,
            created_at: DateTime::<Utc>::default(),
            updated_at: DateTime::<Utc>::default(),
        }
    }

    #[test]
    fn build_tree_nests_children_under_their_parent() {
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let grandchild = Uuid::new_v4();

        let all = vec![
            dept(root, None, "root", 0),
            dept(child, Some(root), "child", 0),
            dept(grandchild, Some(child), "grandchild", 0),
        ];

        let tree = build_tree(&all);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "root");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "child");
        assert_eq!(tree[0].children[0].children.len(), 1);
        assert_eq!(tree[0].children[0].children[0].name, "grandchild");
    }

    #[test]
    fn build_tree_sorts_siblings_by_sort_order() {
        let root = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        let all = vec![
            dept(root, None, "root", 0),
            dept(a, Some(root), "a", 2),
            dept(b, Some(root), "b", 0),
            dept(c, Some(root), "c", 1),
        ];

        let tree = build_tree(&all);
        let names: Vec<&str> = tree[0].children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn build_tree_treats_orphan_nodes_as_roots() {
        // 父节点不在集合内 → 当根返回（与菜单树同一行为）
        let orphan = Uuid::new_v4();
        let missing_parent = Uuid::new_v4();

        let all = vec![dept(orphan, Some(missing_parent), "orphan", 0)];

        let tree = build_tree(&all);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "orphan");
    }

    #[test]
    fn build_tree_drops_cycles_instead_of_looping_forever() {
        // A → B → A 的环：环上无一为根，整支被剪掉
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        let all = vec![dept(a, Some(b), "a", 0), dept(b, Some(a), "b", 0)];

        let tree = build_tree(&all);
        assert!(tree.is_empty(), "环上的节点不该出现在树里");
    }
}
