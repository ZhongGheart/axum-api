//! 数据库连接池管理
//!
//! v0.2 起只维护单一主库连接池：此前的读写分离配置从未被读路径使用，
//! 属于"配置存在但无效果"的死能力，已连同 `DATABASE_READ_URL` 一并删除。

use sqlx::PgPool;

use crate::config::DatabaseConfig;
use crate::error::AppError;

/// 数据库连接池
#[derive(Debug, Clone)]
pub struct DatabasePool {
    writer: PgPool,
}

impl DatabasePool {
    /// 初始化数据库连接池
    pub async fn new(config: &DatabaseConfig) -> Result<Self, AppError> {
        use sqlx::postgres::PgPoolOptions;

        let writer = PgPoolOptions::new()
            .max_connections(config.max_size)
            .acquire_timeout(std::time::Duration::from_secs(
                config.connect_timeout_seconds,
            ))
            .connect(&config.write_url)
            .await
            .map_err(|e| AppError::InternalServerError(format!("数据库连接失败: {e}")))?;

        Ok(Self { writer })
    }

    /// 执行数据库迁移
    ///
    /// 迁移文件在编译期通过 `sqlx::migrate!` 嵌入二进制，
    /// 因此运行镜像无需携带 `migrations/` 目录。
    pub async fn run_migrations(&self) -> Result<(), AppError> {
        sqlx::migrate!("./migrations")
            .run(&self.writer)
            .await
            .map_err(|e| AppError::InternalServerError(format!("数据库迁移失败: {e}")))?;
        Ok(())
    }

    /// 获取写库连接池
    pub fn writer(&self) -> &PgPool {
        &self.writer
    }
}
