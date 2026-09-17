//! 数据库读写分离池管理
//!
//! 基于 SQLx PgPool，提供读写分离连接池。
//! 读取操作使用读库连接池，写入操作使用写库连接池。

use sqlx::PgPool;

use crate::config::DatabaseConfig;
use crate::error::AppError;

/// 读写分离数据库池
#[derive(Debug, Clone)]
pub struct DatabasePool {
    /// 写库连接池（主库）
    pub writer: PgPool,
    /// 读库连接池（从库，与写库相同则回退）
    pub reader: PgPool,
    /// 是否真正分离
    pub use_read_replica: bool,
}

impl DatabasePool {
    /// 初始化数据库连接池
    pub async fn new(config: &DatabaseConfig) -> Result<Self, AppError> {
        use sqlx::postgres::PgPoolOptions;

        let writer = PgPoolOptions::new()
            .max_connections(config.max_size)
            .acquire_timeout(std::time::Duration::from_secs(config.connect_timeout_seconds))
            .connect(&config.write_url)
            .await
            .map_err(|e| AppError::InternalServerError(format!("写库连接失败: {e}")))?;

        let (reader, use_read_replica) = if let Some(ref read_url) = config.read_url {
            let read_pool = match PgPoolOptions::new()
                .max_connections(config.read_max_size.unwrap_or(config.max_size))
                .acquire_timeout(std::time::Duration::from_secs(config.connect_timeout_seconds))
                .connect(read_url)
                .await
            {
                Ok(pool) => pool,
                Err(e) => {
                    tracing::warn!("读库连接失败，回退到写库: {e}");
                    writer.clone()
                }
            };
            (read_pool, true)
        } else {
            (writer.clone(), false)
        };

        Ok(Self {
            writer,
            reader,
            use_read_replica,
        })
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

    /// 获取读连接池
    pub fn reader(&self) -> &PgPool {
        if self.use_read_replica {
            &self.reader
        } else {
            &self.writer
        }
    }

    /// 获取写连接池
    pub fn writer(&self) -> &PgPool {
        &self.writer
    }
}
