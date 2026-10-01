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

    /// 查询多个角色可见的**导航菜单树**
    ///
    /// - 合并用户所有角色的菜单并去重
    /// - 只返回 `is_visible = true` 且非按钮（`type <> 'button'`）的节点，
    ///   按钮型菜单是权限标记，不应出现在导航里
    /// - 排序沿用 `sort_order`
    pub async fn find_tree_for_roles(&self, role_ids: &[Uuid]) -> Result<Vec<MenuNode>, AppError> {
        if role_ids.is_empty() {
            return Ok(Vec::new());
        }

        let menus = sqlx::query_as::<_, Menu>(
            r#"
            SELECT DISTINCT m.id, m.parent_id, m.name, m.path, m.component, m.icon,
                   m.sort_order, m.type, m.permission, m.is_visible, m.created_at, m.updated_at
            FROM menus m
            JOIN role_menus rm ON rm.menu_id = m.id
            WHERE rm.role_id = ANY($1)
              AND m.is_visible = TRUE
              AND m.type <> 'button'
            ORDER BY m.sort_order ASC
            "#,
        )
        .bind(role_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户菜单失败: {e}")))?;

        Ok(build_tree(menus, None))
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

    /// 查询给定角色集合拥有的权限码
    ///
    /// 权限码是 `type = 'button'` 菜单行的 `permission` 列，经 `role_menus` 授权。
    /// 单条索引 JOIN 完成，无 N+1：
    ///
    /// ```sql
    /// SELECT DISTINCT m.permission
    /// FROM menus m
    /// JOIN role_menus rm ON rm.menu_id = m.id
    /// JOIN roles r      ON r.id = rm.role_id
    /// WHERE r.name = ANY($1) AND m.type = 'button'
    ///   AND m.permission IS NOT NULL AND m.permission <> ''
    /// ```
    ///
    /// 刻意不做缓存：撤销角色菜单授权后必须立即生效，
    /// 避免 TTL 窗口内出现已撤权仍可调用的情况。
    pub async fn find_permission_codes(
        &self,
        role_names: &[String],
    ) -> Result<Vec<String>, AppError> {
        if role_names.is_empty() {
            return Ok(Vec::new());
        }

        let codes = sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT m.permission
            FROM menus m
            JOIN role_menus rm ON rm.menu_id = m.id
            JOIN roles r      ON r.id = rm.role_id
            WHERE r.name = ANY($1)
              AND m.type = 'button'
              AND m.permission IS NOT NULL
              AND m.permission <> ''
            ORDER BY m.permission ASC
            "#,
        )
        .bind(role_names)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户权限码失败: {e}")))?;

        Ok(codes)
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

    /// 删除菜单
    ///
    /// 子节点与 `role_menus` 的清理**交给数据库外键级联**：
    /// `menus.parent_id` 与 `role_menus.menu_id` 都声明了 `ON DELETE CASCADE`，
    /// 单条 DELETE 即原子地删掉整棵子树及其全部角色授权。
    ///
    /// 早期实现在这里手写递归删除子节点，且每条语句都用 `.ok()` 吞掉错误：
    /// 那些语句既多余（外键已经级联），又把真实错误藏了起来。
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        // 不存在的菜单返回 404，而不是"删除成功"——UI 上的删除按钮需要能区分两者
        let rows = sqlx::query("DELETE FROM menus WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除菜单失败: {e}")))?;

        if rows.rows_affected() == 0 {
            return Err(AppError::NotFound("菜单不存在".into()));
        }
        Ok(())
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
    ///
    /// 全量替换语义：先清空该角色的全部授权，再写入新集合。
    /// 三步在同一事务内，**任何一步失败都整体回滚**。
    ///
    /// 早期实现对 DELETE / INSERT 都用 `.ok()` 吞掉错误却仍然 `commit()`，后果是：
    /// - 撤销静默失效——取消勾选、保存成功，权限其实还在
    /// - 传入"合法 + 非法"混合 ID 时静默**部分**授权，却返回"权限分配成功"
    ///
    /// 授权写路径必须"要么完整成功、要么整体失败"：半吊子的授权比没有授权更危险，
    /// 因为管理员会以为撤销已经生效。
    pub async fn assign_role_menus(
        &self,
        role_id: Uuid,
        menu_ids: &[Uuid],
    ) -> Result<(), AppError> {
        // 角色不存在要给出明确的 404，而不是等外键约束报错
        let role_exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM roles WHERE id = $1")
            .bind(role_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;
        if role_exists.is_none() {
            return Err(AppError::NotFound("角色不存在".into()));
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query("DELETE FROM role_menus WHERE role_id = $1")
            .bind(role_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清空角色原有授权失败: {e}")))?;

        if !menu_ids.is_empty() {
            // 单条 `INSERT ... SELECT unnest` 取代逐条循环：一次往返而非 N 次，
            // 且任一 ID 非法（外键不存在）会让整条语句失败 → 事务回滚，
            // 不会留下"只授权了一半"的中间态。
            sqlx::query(
                "INSERT INTO role_menus (role_id, menu_id)
                 SELECT $1, unnest($2::uuid[])
                 ON CONFLICT DO NOTHING",
            )
            .bind(role_id)
            .bind(menu_ids)
            .execute(&mut *tx)
            .await
            .map_err(map_menu_fk_error)?;
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

/// 把菜单授权写入时的数据库错误翻译成可操作的提示
///
/// `role_menus.menu_id` 有指向 `menus(id)` 的外键，因此传入不存在的菜单 ID 会触发
/// `23503`（foreign_key_violation）。这类错误是**调用方的问题**（传错了 ID），
/// 应返回 400 而不是让人从"服务器内部错误: ..."里猜。
fn map_menu_fk_error(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.code().as_deref() == Some("23503") {
            return AppError::BadRequest("提交的菜单 ID 不存在，请刷新后重试".into());
        }
    }
    AppError::InternalServerError(format!("写入角色授权失败: {e}"))
}
