//! 部门数据访问层（Repository）
//!
//! 封装对 `departments` 表的所有数据库操作。
//!
//! 树形结构用应用层 [`crate::model::department::build_tree`] 构建，
//! 与菜单树同一模式。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::department::{Department, DepartmentUser};
use sqlx::PgPool;

/// `departments` 表映射成 [`Department`] 时必须**恰好**取到的列
///
/// 与 `repository::user::USER_COLUMNS` 同理：集中成常量，加列只改这一处。
pub const DEPARTMENT_COLUMNS: &str =
    "id, parent_id, name, description, sort_order, created_at, updated_at";

/// 部门数据访问层
#[derive(Debug, Clone)]
pub struct DepartmentRepository {
    pool: PgPool,
}

impl DepartmentRepository {
    /// 创建新的 DepartmentRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 数据库连接池访问器
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 列出全部部门（扁平，按 sort_order 升序）
    ///
    /// 返回扁平列表而不是树：树由应用层 [`crate::model::department::build_tree`]
    /// 构建，与菜单树同一模式。
    pub async fn list_all(&self) -> Result<Vec<Department>, AppError> {
        sqlx::query_as::<_, Department>(&format!(
            r#"
            SELECT {DEPARTMENT_COLUMNS}
            FROM departments
            ORDER BY sort_order ASC, name ASC
            "#
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询部门列表失败: {e}")))
    }

    /// 按 ID 查部门；不存在返回 `None`
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<Department>, AppError> {
        sqlx::query_as::<_, Department>(&format!(
            r#"
            SELECT {DEPARTMENT_COLUMNS}
            FROM departments
            WHERE id = $1
            "#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询部门失败: {e}")))
    }

    /// 统计某部门的**直接**子部门数
    ///
    /// 用于删除前检查：`ON DELETE RESTRICT` 会在数据库层面阻止删除有子部门的
    /// 节点，但服务层先检查一次能给出可操作的错误提示（"请先移动或删除子部门"），
    /// 而不是让数据库错误冒到 500。
    pub async fn count_children(&self, id: Uuid) -> Result<i64, AppError> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM departments WHERE parent_id = $1
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("统计子部门数失败: {e}")))?;
        Ok(count)
    }

    /// 判断 `candidate` 是否是 `ancestor` 的子孙（含自身）
    ///
    /// 用于移动节点时的循环防护：不能把一个节点移动到自己的子孙下面。
    ///
    /// 用递归 CTE 而不是应用层递归：部门数量级在几十到几百，
    /// 但递归 CTE 在数据库层面做更高效，且能处理任意深度。
    ///
    /// **环的处理**：如果数据里存在环，递归 CTE 会无限循环。
    /// 因此加 `depth` 上限（100 层足够任何真实组织结构），
    /// 超过上限就返回 `false`（"不是子孙"），让移动操作被拒绝。
    pub async fn is_descendant(&self, ancestor: Uuid, candidate: Uuid) -> Result<bool, AppError> {
        // 自己不算自己的子孙
        if ancestor == candidate {
            return Ok(false);
        }

        let found: Option<bool> = sqlx::query_scalar(
            r#"
            WITH RECURSIVE subtree AS (
                SELECT id, parent_id, 1 AS depth
                FROM departments
                WHERE id = $1
                UNION ALL
                SELECT d.id, d.parent_id, s.depth + 1
                FROM departments d
                JOIN subtree s ON d.parent_id = s.id
                WHERE s.depth < 100
            )
            SELECT EXISTS(SELECT 1 FROM subtree WHERE id = $2)
            "#,
        )
        .bind(ancestor)
        .bind(candidate)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("判断子孙关系失败: {e}")))?;

        Ok(found.unwrap_or(false))
    }

    /// 新建部门
    pub async fn create(
        &self,
        parent_id: Option<Uuid>,
        name: &str,
        description: Option<&str>,
        sort_order: i32,
    ) -> Result<Department, AppError> {
        sqlx::query_as::<_, Department>(&format!(
            r#"
            INSERT INTO departments (parent_id, name, description, sort_order)
            VALUES ($1, $2, $3, $4)
            RETURNING {DEPARTMENT_COLUMNS}
            "#
        ))
        .bind(parent_id)
        .bind(name)
        .bind(description)
        .bind(sort_order)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("新建部门失败: {e}")))
    }

    /// 修改部门（名称 / 描述 / 排序）
    ///
    /// **不修改 `parent_id`**：移动节点是独立操作（[`Self::move`]），
    /// 因为移动需要循环防护，而修改名称不需要。
    pub async fn update(
        &self,
        id: Uuid,
        name: &str,
        description: Option<&str>,
        sort_order: i32,
    ) -> Result<Department, AppError> {
        sqlx::query_as::<_, Department>(&format!(
            r#"
            UPDATE departments
            SET name = $2, description = $3, sort_order = $4
            WHERE id = $1
            RETURNING {DEPARTMENT_COLUMNS}
            "#
        ))
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(sort_order)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("修改部门失败: {e}")))
    }

    /// 移动部门（改 `parent_id`）
    ///
    /// 循环防护由服务层在调用前完成（[`Self::is_descendant`]）。
    /// 这里只执行 UPDATE，不做检查——检查与写入分开，
    /// 是为了让服务层能在检查失败时给出可操作的错误提示。
    pub async fn set_parent(
        &self,
        id: Uuid,
        new_parent_id: Option<Uuid>,
    ) -> Result<Department, AppError> {
        sqlx::query_as::<_, Department>(&format!(
            r#"
            UPDATE departments
            SET parent_id = $2
            WHERE id = $1
            RETURNING {DEPARTMENT_COLUMNS}
            "#
        ))
        .bind(id)
        .bind(new_parent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("移动部门失败: {e}")))
    }

    /// 删除部门
    ///
    /// `ON DELETE RESTRICT` 会在有子部门时阻止删除。
    /// 服务层会先检查有没有子部门，有就拒绝并提示。
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let result = sqlx::query(
            r#"
            DELETE FROM departments WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("删除部门失败: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("部门不存在".to_string()));
        }
        Ok(())
    }

    /// 列出某部门下的用户
    pub async fn list_users(&self, dept_id: Uuid) -> Result<Vec<DepartmentUser>, AppError> {
        sqlx::query_as::<_, DepartmentUser>(
            r#"
            SELECT id, username, display_name, email, is_active
            FROM users
            WHERE dept_id = $1
            ORDER BY username ASC
            "#,
        )
        .bind(dept_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询部门用户失败: {e}")))
    }

    /// 统计某部门下的用户数
    pub async fn count_users(&self, dept_id: Uuid) -> Result<i64, AppError> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM users WHERE dept_id = $1
            "#,
        )
        .bind(dept_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("统计部门用户数失败: {e}")))?;
        Ok(count)
    }
}
