//! 操作日志数据访问层
//!
//! 只提供读取能力：写入由 `middleware::audit_log` 异步完成。

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
}
