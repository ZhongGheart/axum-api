//! 用户数据访问层（Repository）
//!
//! 封装对 `users` 表的所有数据库操作。
//! 使用 SQLx 异步连接池，通过 SQL 语句直接操作数据库（非 ORM 方式）。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::User;
use sqlx::PgPool;

/// 用户仓储
///
/// 提供用户相关的数据库 CRUD 操作。
#[derive(Debug, Clone)]
pub struct UserRepository {
    /// SQLx PostgreSQL 连接池
    pub pool: PgPool,
}

impl UserRepository {
    /// 创建新的 UserRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 根据用户 ID 查找用户
    ///
    /// # Arguments
    ///
    /// * `id` - 用户 UUID
    ///
    /// # Returns
    ///
    /// 找到返回 `User`，未找到返回 `AppError::NotFound`。
    pub async fn find_by_id(&self, id: Uuid) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
    }

    /// 根据用户名查找用户（精确匹配）
    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            WHERE username = $1
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 根据用户名或邮箱查找用户（用于登录）
    ///
    /// 支持用户名或邮箱两种方式的登录查询。
    pub async fn find_by_username_or_email(&self, input: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            WHERE username = $1 OR email = $1
            "#,
        )
        .bind(input)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 根据邮箱查找用户
    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 查询所有用户（分页）
    pub async fn list_all(&self, page: i64, page_size: i64) -> Result<(Vec<User>, i64), AppError> {
        let offset = (page - 1) * page_size;
        let users = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户列表失败: {e}")))?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询用户总数失败: {e}")))?;

        Ok((users, total.0))
    }

    /// 更新用户信息（用户名、邮箱、是否激活）
    pub async fn update(
        &self,
        id: Uuid,
        username: &str,
        email: &str,
        role: &str,
        is_active: bool,
    ) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            UPDATE users
            SET username = $2, email = $3, role = $4, is_active = $5
            WHERE id = $1
            RETURNING id, username, email, password_hash, role, is_active, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(role)
        .bind(is_active)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(target: "repository", "更新用户失败 (id={}): {:?}", id, e);
            if let Some(pg_err) = e.as_database_error() {
                if let Some(constraint) = pg_err.constraint() {
                    if constraint == "users_username_key" {
                        return AppError::Conflict("用户名已被占用".to_string());
                    }
                    if constraint == "users_email_key" {
                        return AppError::Conflict("邮箱已被占用".to_string());
                    }
                }
            }
            AppError::InternalServerError(format!("更新用户失败: {e}"))
        })
    }

    /// 更新密码哈希（用于改密与 v0.1 旧口令格式升级）
    pub async fn update_password_hash(&self, id: Uuid, password_hash: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("更新密码失败: {e}")))?;
        Ok(())
    }

    /// 删除用户
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除用户失败: {e}")))?;
        Ok(())
    }

    /// 创建新用户
    ///
    /// # Arguments
    ///
    /// * `id` - 新用户 UUID
    /// * `username` - 用户名
    /// * `email` - 电子邮箱
    /// * `password_hash` - Argon2 哈希后的密码
    ///
    /// # Returns
    ///
    /// 返回创建成功的用户记录。
    pub async fn create(
        &self,
        id: Uuid,
        username: &str,
        email: &str,
        password_hash: &str,
    ) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (id, username, email, password_hash, role, is_active)
            VALUES ($1, $2, $3, $4, 'user', true)
            RETURNING id, username, email, password_hash, role, is_active, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // 检查是否唯一约束冲突
            if let Some(pg_err) = e.as_database_error() {
                if let Some(constraint) = pg_err.constraint() {
                    if constraint == "users_username_key" {
                        return AppError::Conflict("用户名已被注册".to_string());
                    }
                    if constraint == "users_email_key" {
                        return AppError::Conflict("邮箱已被注册".to_string());
                    }
                }
            }
            AppError::InternalServerError(format!("创建用户失败: {e}"))
        })
    }
}
