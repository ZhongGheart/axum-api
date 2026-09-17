//! 角色权限服务层
//!
//! 封装角色分配、初始化种子数据等业务逻辑。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::User;
use crate::repository::role::RoleRepository;
use crate::utils::password::hash_password;

/// 角色权限服务
#[derive(Debug, Clone)]
pub struct RbacService {
    /// 角色仓储
    pub role_repo: RoleRepository,
    /// 数据库连接池（用于执行原始 SQL）
    pool: sqlx::PgPool,
}

impl RbacService {
    /// 创建新的 RbacService 实例
    pub fn new(role_repo: RoleRepository, pool: sqlx::PgPool) -> Self {
        Self { role_repo, pool }
    }

    /// 初始化默认角色和超级管理员
    ///
    /// 幂等操作：仅当 `roles` 表为空时执行。
    pub async fn init_defaults(&self) -> Result<(), AppError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM roles")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;

        if count.0 > 0 {
            tracing::info!("RBAC 已初始化，跳过种子数据");
            return Ok(());
        }

        tracing::info!("开始初始化 RBAC 种子数据...");

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

        // 1. 创建默认角色
        sqlx::query(
            r#"
            INSERT INTO roles (name, description) VALUES
                ('admin', '系统管理员，拥有所有权限'),
                ('user',  '普通用户，基础访问权限')
            ON CONFLICT (name) DO NOTHING
            "#,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建默认角色失败: {e}")))?;

        // 2. 查找角色 ID
        let admin_role: (Uuid,) = sqlx::query_as(
            "SELECT id FROM roles WHERE name = 'admin'",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询 admin 角色失败: {e}")))?;

        let user_role: (Uuid,) = sqlx::query_as(
            "SELECT id FROM roles WHERE name = 'user'",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询 user 角色失败: {e}")))?;

        // 3. 创建默认超级管理员（如不存在）
        let admin_user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            WHERE username = 'admin'
            "#,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询 admin 用户失败: {e}")))?;

        // 明文 → Argon2（v0.2 起取消客户端 SHA-256 预哈希）
        let password_hash = hash_password("admin123")
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let admin_id = if let Some(admin) = admin_user {
            // admin 已存在，更新密码哈希（兼容旧格式迁移）
            sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
                .bind(&password_hash)
                .bind(admin.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| AppError::InternalServerError(format!("更新 admin 密码失败: {e}")))?;
            admin.id
        } else {
            let new_admin: (Uuid,) = sqlx::query_as(
                r#"
                INSERT INTO users (username, email, password_hash, role, is_active)
                VALUES ('admin', 'admin@example.com', $1, 'admin', true)
                RETURNING id
                "#,
            )
            .bind(&password_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("创建 admin 用户失败: {e}")))?;

            new_admin.0
        };

        // 4. 分配 admin + user 角色
        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
            ON CONFLICT (user_id, role_id) DO NOTHING
            "#,
        )
        .bind(admin_id)
        .bind(admin_role.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配 admin 角色失败: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
            ON CONFLICT (user_id, role_id) DO NOTHING
            "#,
        )
        .bind(admin_id)
        .bind(user_role.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配 user 角色失败: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;

        tracing::info!("RBAC 种子数据初始化完成");

        // 5. 将已有 users.role 同步到 user_roles
        self.sync_existing_users().await?;

        Ok(())
    }

    /// 将现有 `users.role` 同步到 `user_roles`
    async fn sync_existing_users(&self) -> Result<(), AppError> {
        let users = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, email, password_hash, role, is_active, created_at, updated_at
            FROM users
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户列表失败: {e}")))?;

        for user in users {
            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM user_roles WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询用户角色失败: {e}")))?;

            if count.0 > 0 {
                continue;
            }

            let role_name = match user.role {
                crate::model::user::Role::Admin => "admin",
                crate::model::user::Role::User => "user",
            };

            self.role_repo
                .assign_role_to_user(user.id, role_name)
                .await?;
        }

        Ok(())
    }
}
