//! 菜单数据访问层

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{Menu, MenuNode};

/// 菜单仓储
#[derive(Debug, Clone)]
pub struct MenuRepository {
    pool: PgPool,
}

impl MenuRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 查询所有菜单（平坦列表）
    pub async fn find_all(&self) -> Result<Vec<Menu>, AppError> {
        sqlx::query_as::<_, Menu>(
            "SELECT id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible, created_at, updated_at FROM menus ORDER BY sort_order ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询菜单失败: {e}")))
    }

    /// 构建菜单树
    pub async fn find_tree(&self) -> Result<Vec<MenuNode>, AppError> {
        let all = self.find_all().await?;
        Ok(build_tree(all, None))
    }

    /// 根据角色 ID 查询菜单 ID 列表
    pub async fn find_menu_ids_by_role(&self, role_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let ids =
            sqlx::query_scalar::<_, Uuid>("SELECT menu_id FROM role_menus WHERE role_id = $1")
                .bind(role_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::InternalServerError(format!("查询角色菜单失败: {e}")))?;
        Ok(ids)
    }

    /// 根据角色 ID 查询菜单树（仅返回有权限的节点）
    pub async fn find_tree_by_role(&self, role_id: Uuid) -> Result<Vec<MenuNode>, AppError> {
        let menu_ids = self.find_menu_ids_by_role(role_id).await?;
        let all = self.find_all().await?;
        let filtered: Vec<Menu> = all
            .into_iter()
            .filter(|m| menu_ids.contains(&m.id))
            .collect();
        Ok(build_tree(filtered, None))
    }

    /// 新增菜单
    pub async fn create(&self, menu: &Menu) -> Result<Menu, AppError> {
        sqlx::query_as::<_, Menu>(
            r#"
            INSERT INTO menus (id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible, created_at, updated_at
            "#,
        )
        .bind(menu.id)
        .bind(menu.parent_id)
        .bind(&menu.name)
        .bind(&menu.path)
        .bind(&menu.component)
        .bind(&menu.icon)
        .bind(menu.sort_order)
        .bind(&menu.r#type)
        .bind(&menu.permission)
        .bind(menu.is_visible)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建菜单失败: {e}")))
    }

    /// 更新菜单
    pub async fn update(
        &self,
        id: Uuid,
        fields: &crate::model::UpdateMenuRequest,
    ) -> Result<Menu, AppError> {
        let menu = self.find_by_id(id).await?;
        let parent_id = fields.parent_id.or(menu.parent_id);
        let name = fields.name.as_deref().unwrap_or(&menu.name);
        let path = fields.path.as_deref().or(menu.path.as_deref());
        let component = fields.component.as_deref().or(menu.component.as_deref());
        let icon = fields.icon.as_deref().or(menu.icon.as_deref());
        let sort_order = fields.sort_order.unwrap_or(menu.sort_order);
        let r#type = fields.r#type.as_deref().unwrap_or(&menu.r#type);
        let permission = fields.permission.as_deref().or(menu.permission.as_deref());
        let is_visible = fields.is_visible.unwrap_or(menu.is_visible);

        sqlx::query_as::<_, Menu>(
            r#"
            UPDATE menus SET parent_id=$2, name=$3, path=$4, component=$5, icon=$6,
                sort_order=$7, type=$8, permission=$9, is_visible=$10
            WHERE id=$1
            RETURNING id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible, created_at, updated_at
            "#,
        )
        .bind(id).bind(parent_id).bind(name).bind(path).bind(component)
        .bind(icon).bind(sort_order).bind(r#type).bind(permission).bind(is_visible)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("更新菜单失败: {e}")))
    }

    /// 删除菜单（级联删除子节点 + role_menus）
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query("DELETE FROM role_menus WHERE menu_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .ok();
        // 递归删除子节点
        let children: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM menus WHERE parent_id = $1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
        for (cid,) in children {
            sqlx::query("DELETE FROM role_menus WHERE menu_id = $1")
                .bind(cid)
                .execute(&mut *tx)
                .await
                .ok();
            sqlx::query("DELETE FROM menus WHERE id = $1")
                .bind(cid)
                .execute(&mut *tx)
                .await
                .ok();
        }
        sqlx::query("DELETE FROM menus WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除菜单失败: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }

    /// 根据 ID 查询
    pub async fn find_by_id(&self, id: Uuid) -> Result<Menu, AppError> {
        sqlx::query_as::<_, Menu>(
            "SELECT id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible, created_at, updated_at FROM menus WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询菜单失败: {e}")))?
        .ok_or_else(|| AppError::NotFound("菜单不存在".into()))
    }

    /// 分配角色菜单权限（全量替换）
    pub async fn assign_role_menus(
        &self,
        role_id: Uuid,
        menu_ids: &[Uuid],
    ) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query("DELETE FROM role_menus WHERE role_id = $1")
            .bind(role_id)
            .execute(&mut *tx)
            .await
            .ok();
        for mid in menu_ids {
            sqlx::query(
                "INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(role_id)
            .bind(mid)
            .execute(&mut *tx)
            .await
            .ok();
        }
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }
}

/// 递归构建菜单树
fn build_tree(all: Vec<Menu>, parent: Option<Uuid>) -> Vec<MenuNode> {
    all.iter()
        .filter(|m| m.parent_id == parent)
        .map(|m| {
            let mut node = MenuNode::from(m.clone());
            node.children = build_tree(all.clone(), Some(m.id));
            node
        })
        .collect()
}
