//! 角色权限数据访问层
//!
//! 封装对 `roles` 和 `user_roles` 表的数据库操作。
//! 使用事务保证关联数据的一致性（建角色 + 分配用户 + 初始化种子数据）。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::RoleRow;
use sqlx::PgPool;

/// 角色仓储
#[derive(Debug, Clone)]
pub struct RoleRepository {
    pool: PgPool,
}

#[allow(dead_code)]
impl RoleRepository {
    /// 创建新的 RoleRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 按名称查找角色
    pub async fn find_by_name(&self, name: &str) -> Result<Option<RoleRow>, AppError> {
        sqlx::query_as::<_, RoleRow>(
            r#"
            SELECT id, name, description, created_at
            FROM roles
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))
    }

    /// 查询所有角色（含用户数）
    pub async fn list_all(&self) -> Result<Vec<(RoleRow, i64)>, AppError> {
        let rows = sqlx::query_as::<_, (Uuid, String, Option<String>, DateTime<Utc>, i64)>(
            r#"
            SELECT r.id, r.name, r.description, r.created_at,
                   COALESCE(ur_cnt.cnt, 0) AS user_count
            FROM roles r
            LEFT JOIN (
                SELECT role_id, COUNT(*) AS cnt FROM user_roles GROUP BY role_id
            ) ur_cnt ON ur_cnt.role_id = r.id
            ORDER BY r.created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询角色列表失败: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|(id, name, description, created_at, user_count)| {
                (
                    RoleRow { id, name, description, created_at },
                    user_count,
                )
            })
            .collect())
    }

    /// 查询用户拥有的所有角色
    pub async fn find_roles_by_user_id(&self, user_id: Uuid) -> Result<Vec<String>, AppError> {
        let roles = sqlx::query_scalar::<_, String>(
            r#"
            SELECT r.name
            FROM user_roles ur
            JOIN roles r ON r.id = ur.role_id
            WHERE ur.user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户角色失败: {e}")))?;

        Ok(roles)
    }

    /// 为用户分配角色（使用事务）
    pub async fn assign_role_to_user(
        &self,
        user_id: Uuid,
        role_name: &str,
    ) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

        let role = sqlx::query_as::<_, RoleRow>(
            r#"
            SELECT id, name, description, created_at
            FROM roles
            WHERE name = $1
            "#,
        )
        .bind(role_name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("角色不存在: {role_name}")))?;

        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, role_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(role.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配角色失败: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;

        Ok(())
    }

    /// 批量分配默认角色给新用户（事务保证）
    pub async fn assign_default_roles(
        &self,
        user_id: Uuid,
        role_names: &[&str],
    ) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

        for role_name in role_names {
            let role = sqlx::query_as::<_, RoleRow>(
                r#"
                SELECT id, name, description, created_at
                FROM roles
                WHERE name = $1
                "#,
            )
            .bind(role_name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("角色不存在: {role_name}")))?;

            sqlx::query(
                r#"
                INSERT INTO user_roles (user_id, role_id)
                VALUES ($1, $2)
                ON CONFLICT (user_id, role_id) DO NOTHING
                "#,
            )
            .bind(user_id)
            .bind(role.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("分配角色失败: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;

        Ok(())
    }
}
