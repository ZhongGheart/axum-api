//! 操作日志数据访问层
//!
//! 只提供读取能力：写入由 `middleware::audit_log` 异步完成。

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::model::AuditLog;
use crate::utils::pagination::{PaginatedResponse, PaginationParams};

/// 操作日志仓储
#[derive(Debug, Clone)]
pub struct AuditLogRepository {
    pool: PgPool,
}

impl AuditLogRepository {
    /// 创建新的 AuditLogRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 分页查询操作日志
    ///
    /// 表名与排序列均为编译期常量，不存在拼接用户输入的情况。
    pub async fn paginate(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<AuditLog>, AppError> {
        const ALLOWED_SORT_FIELDS: [&str; 4] = ["created_at", "username", "action", "status_code"];

        let page = params.get_page();
        let page_size = params.get_page_size();
        let offset = params.get_offset();
        let order_sql = params.get_order_sql(&ALLOWED_SORT_FIELDS);

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志总数失败: {e}")))?;

        let sql = format!(
            "SELECT id, user_id, username, action, method, path, params, result, \
             status_code, client_ip, duration_ms, created_at \
             FROM audit_logs ORDER BY {order_sql} LIMIT $1 OFFSET $2"
        );

        let items = sqlx::query_as::<_, AuditLog>(&sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

        Ok(PaginatedResponse::new(items, total.0, page, page_size))
    }

    /// 删除 `cutoff` 之前的日志，返回实际删除行数
    ///
    /// **分批**删除，而不是一条 `DELETE FROM audit_logs WHERE created_at < $1`：
    /// 一次性删几十万行会长时间持锁并把 WAL 撑爆，期间其他事务只能干等。
    /// 分批把锁持有时间切碎；某批没删满即说明已删到 cutoff 附近，提前收工。
    ///
    /// 子查询按 `created_at` 升序取最旧的一批，
    /// 正好反向扫描 `idx_audit_logs_created`，不必全表排序。
    pub async fn delete_older_than(
        &self,
        cutoff: DateTime<Utc>,
        batch_size: i64,
        max_batches: u32,
    ) -> Result<u64, AppError> {
        let batch_size = batch_size.max(1);
        let mut total_deleted: u64 = 0;

        for _ in 0..max_batches {
            let deleted = sqlx::query(
                r#"
                DELETE FROM audit_logs WHERE id IN (
                    SELECT id FROM audit_logs
                    WHERE created_at < $1
                    ORDER BY created_at
                    LIMIT $2
                )
                "#,
            )
            .bind(cutoff)
            .bind(batch_size)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清理过期操作日志失败: {e}")))?
            .rows_affected();

            total_deleted += deleted;
            if deleted < batch_size as u64 {
                break;
            }
        }

        Ok(total_deleted)
    }
}
