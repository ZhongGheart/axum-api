//! 通用 CRUD 模板函数
//!
//! 封装常见的数据库 CRUD 操作，减少重复代码。
//! 所有方法直接返回 `Result<T, AppError>`，与现有错误处理兼容。

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::utils::pagination::{PaginationParams, PaginatedResponse};

/// 通用 CRUD 操作接口
pub struct CrudTemplate;

impl CrudTemplate {
    /// 分页查询
    ///
    /// # Arguments
    /// * `pool` - 数据库连接池
    /// * `table` - 表名
    /// * `params` - 分页参数
    /// * `allowed_sort_fields` - 允许排序的字段列表
    /// * `where_clause` - 可选的 WHERE 条件（不含 WHERE 关键字）
    ///
    /// 返回统一分页响应。
    pub async fn paginate<T>(
        pool: &PgPool,
        table: &str,
        params: &PaginationParams,
        allowed_sort_fields: &[&str],
        where_clause: Option<&str>,
    ) -> Result<PaginatedResponse<T>, AppError>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin + serde::Serialize,
    {
        let page = params.get_page();
        let page_size = params.get_page_size();
        let offset = params.get_offset();
        let order_sql = params.get_order_sql(allowed_sort_fields);

        let where_sql = where_clause
            .map(|w| format!("WHERE {}", w))
            .unwrap_or_default();

        let count_sql = format!("SELECT COUNT(*) FROM {} {}", table, where_sql);
        let data_sql = format!(
            "SELECT * FROM {} {} ORDER BY {} LIMIT $1 OFFSET $2",
            table, where_sql, order_sql
        );

        let total: (i64,) = sqlx::query_as(&count_sql)
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询总数失败: {e}")))?;

        let items: Vec<T> = sqlx::query_as(&data_sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("分页查询失败: {e}")))?;

        Ok(PaginatedResponse::new(items, total.0, page, page_size))
    }

    /// 根据 ID 查询
    pub async fn find_by_id<T>(pool: &PgPool, table: &str, id: Uuid) -> Result<T, AppError>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
    {
        let sql = format!("SELECT * FROM {} WHERE id = $1", table);
        sqlx::query_as::<_, T>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询失败: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("{} 中未找到 ID: {}", table, id)))
    }

    /// 删除
    pub async fn delete(pool: &PgPool, table: &str, id: Uuid) -> Result<(), AppError> {
        let sql = format!("DELETE FROM {} WHERE id = $1", table);
        sqlx::query(&sql)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除失败: {e}")))?;
        Ok(())
    }

    /// 批量删除
    pub async fn delete_many(pool: &PgPool, table: &str, ids: &[Uuid]) -> Result<(), AppError> {
        if ids.is_empty() {
            return Ok(());
        }
        // 使用 IN 子句
        let placeholders: Vec<String> = ids.iter().enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();
        let sql = format!("DELETE FROM {} WHERE id IN ({})", table, placeholders.join(","));

        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id);
        }
        query.execute(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("批量删除失败: {e}")))?;
        Ok(())
    }

    /// 检查是否存在
    pub async fn exists(
        pool: &PgPool,
        table: &str,
        field: &str,
        value: &str,
    ) -> Result<bool, AppError> {
        let sql = format!("SELECT COUNT(*) FROM {} WHERE {} = $1", table, field);
        let count: (i64,) = sqlx::query_as(&sql)
            .bind(value)
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询失败: {e}")))?;
        Ok(count.0 > 0)
    }
}
