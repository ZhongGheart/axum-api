//! 操作日志数据访问层
//!
//! 只提供读取能力：写入由 `middleware::audit_log` 异步完成。

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder};

use crate::error::AppError;
use crate::model::AuditLog;
use crate::utils::pagination::{PaginatedResponse, PaginationParams};

/// 操作日志仓储
#[derive(Debug, Clone)]
pub struct AuditLogRepository {
    pool: PgPool,
}

/// 审计日志筛选条件
///
/// 空串与纯空白一律视为"不过滤"：筛选框清空后前端会发空串，
/// 若当成条件就会筛出零条，看起来像"日志没了"。
#[derive(Debug, Clone, Default)]
pub struct AuditLogFilter {
    pub username: Option<String>,
    pub action: Option<String>,
    pub status_code: Option<i32>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
}

impl AuditLogFilter {
    /// 全为空即"无筛选"。导出用它决定要不要保留行数上限。
    pub fn is_empty(&self) -> bool {
        self.username.is_none()
            && self.action.is_none()
            && self.status_code.is_none()
            && self.start_time.is_none()
            && self.end_time.is_none()
    }
}

fn norm(v: Option<&String>) -> Option<String> {
    v.map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 把筛选条件拼进 WHERE 子句
///
/// **用 `QueryBuilder` 动态拼接，而不是 `($1::text IS NULL OR ...)` 那套恒真写法**：
/// 后者虽然省事，但 Postgres 看见的是"对每列都可能为真"的 OR，
/// 优化器无法把它化成索引扫描——日志表越大越慢。
/// 这里每个条件缺失时**根本不进 SQL**，索引照常可用。
///
/// 片段本身是常量，用户输入一律走绑定参数，不存在拼接用户输入的情况。
fn push_filters<'a>(qb: &mut QueryBuilder<'a, Postgres>, f: &'a AuditLogFilter) {
    if let Some(u) = norm(f.username.as_ref()) {
        qb.push(" AND username ILIKE ")
            .push_bind(format!(
                "%{}%",
                crate::utils::validation::escape_like_pattern(&u)
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(a) = norm(f.action.as_ref()) {
        qb.push(" AND action ILIKE ")
            .push_bind(format!(
                "%{}%",
                crate::utils::validation::escape_like_pattern(&a)
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(code) = f.status_code {
        qb.push(" AND status_code = ").push_bind(code);
    }
    if let Some(t) = f.start_time {
        qb.push(" AND created_at >= ").push_bind(t);
    }
    if let Some(t) = f.end_time {
        qb.push(" AND created_at <= ").push_bind(t);
    }
}

const SELECT_COLS: &str = "id, user_id, username, action, method, path, params, result, \
     status_code, client_ip, duration_ms, created_at";

impl AuditLogRepository {
    /// 创建新的 AuditLogRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 分页查询操作日志（可按条件筛选）
    ///
    /// 表名与排序列均为编译期常量，不存在拼接用户输入的情况；
    /// 筛选值一律走绑定参数。
    ///
    /// 计数与取数**共用同一组筛选条件**——两者一旦不一致，
    /// 就会出现"列表 3 条但 total 500"的分页错乱，
    /// 那比筛选失效更容易让人误判数据规模。
    pub async fn paginate(
        &self,
        params: &PaginationParams,
        filter: &AuditLogFilter,
    ) -> Result<PaginatedResponse<AuditLog>, AppError> {
        const ALLOWED_SORT_FIELDS: [&str; 4] = ["created_at", "username", "action", "status_code"];

        let page = params.get_page();
        let page_size = params.get_page_size();
        let offset = params.get_offset();
        let order_sql = params.get_order_sql(&ALLOWED_SORT_FIELDS);

        let mut count_qb: QueryBuilder<Postgres> =
            QueryBuilder::new("SELECT COUNT(*) FROM audit_logs WHERE TRUE");
        push_filters(&mut count_qb, filter);
        let total: (i64,) = count_qb
            .build_query_as()
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志总数失败: {e}")))?;

        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT {SELECT_COLS} FROM audit_logs WHERE TRUE"));
        push_filters(&mut qb, filter);
        // `order_sql` 来自字段白名单，不含用户输入
        qb.push(format!(" ORDER BY {order_sql} LIMIT "))
            .push_bind(page_size)
            .push(" OFFSET ")
            .push_bind(offset);

        let items = qb
            .build_query_as::<AuditLog>()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

        Ok(PaginatedResponse::new(items, total.0, page, page_size))
    }

    /// 按筛选条件取日志用于导出，返回 `(导出的行, 是否被上限截断)`
    ///
    /// **上限必须连同"是否截断"一起返回**。原实现硬编码 `LIMIT 10000`
    /// 且不告诉任何人，于是用户以为导出了全量，实际只有最新一万条——
    /// 静默截断比报错更糟：报错至少让人知道要缩小范围。
    pub async fn fetch_for_export(
        &self,
        filter: &AuditLogFilter,
        max_rows: i64,
    ) -> Result<(Vec<AuditLog>, bool), AppError> {
        // 多取一行用来判断是否触顶：若恰好取回 max_rows + 1，
        // 说明还有更多数据被砍掉了
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT {SELECT_COLS} FROM audit_logs WHERE TRUE"));
        push_filters(&mut qb, filter);
        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(max_rows + 1);

        let mut rows: Vec<AuditLog> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

        let truncated = rows.len() as i64 > max_rows;
        if truncated {
            rows.truncate(max_rows as usize);
        }
        Ok((rows, truncated))
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
