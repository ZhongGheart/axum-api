//! 通用分页查询封装
//!
//! 标准化分页参数、排序、多条件筛选。
//! 前端和后端统一使用此参数结构。

use serde::{Deserialize, Serialize};

/// 统一分页请求参数
#[derive(Debug, Deserialize, Serialize, Clone, utoipa::ToSchema)]
pub struct PaginationParams {
    /// 页码（从 1 开始）
    pub page: Option<i64>,
    /// 每页条数
    pub page_size: Option<i64>,
    /// 排序字段
    pub sort_by: Option<String>,
    /// 排序方向（asc / desc）
    pub sort_order: Option<String>,
}

impl PaginationParams {
    /// 获取标准化页码（最小 1）
    pub fn get_page(&self) -> i64 {
        self.page.unwrap_or(1).max(1)
    }

    /// 获取标准化每页条数（1-200）
    pub fn get_page_size(&self) -> i64 {
        self.page_size.unwrap_or(10).clamp(1, 200)
    }

    /// 获取偏移量
    pub fn get_offset(&self) -> i64 {
        (self.get_page() - 1) * self.get_page_size()
    }

    /// 获取排序 SQL 片段（带防注入过滤）
    pub fn get_order_sql(&self, allowed_fields: &[&str]) -> String {
        let field = self.sort_by.as_deref().unwrap_or("created_at");
        let field = if allowed_fields.contains(&field) {
            field
        } else {
            "created_at"
        };
        let order = match self.sort_order.as_deref() {
            Some("asc") | Some("ASC") => "ASC",
            _ => "DESC",
        };
        format!("{} {}", field, order)
    }
}

/// 统一分页响应数据
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        let page = page.max(1);
        let page_size = page_size.max(1);
        let total_pages = if total <= 0 {
            1
        } else {
            (total + page_size - 1) / page_size
        };
        Self {
            items,
            total,
            page,
            page_size,
            total_pages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(
        page: Option<i64>,
        page_size: Option<i64>,
        sort_by: Option<&str>,
        sort_order: Option<&str>,
    ) -> PaginationParams {
        PaginationParams {
            page,
            page_size,
            sort_by: sort_by.map(str::to_string),
            sort_order: sort_order.map(str::to_string),
        }
    }

    #[test]
    fn pagination_uses_safe_defaults() {
        let params = params(None, None, None, None);
        assert_eq!(params.get_page(), 1);
        assert_eq!(params.get_page_size(), 10);
        assert_eq!(params.get_offset(), 0);
    }

    #[test]
    fn pagination_clamps_invalid_values() {
        let params = params(Some(0), Some(999), None, None);
        assert_eq!(params.get_page(), 1);
        assert_eq!(params.get_page_size(), 200);
        assert_eq!(params.get_offset(), 0);
    }

    #[test]
    fn pagination_calculates_offset() {
        let params = params(Some(3), Some(20), None, None);
        assert_eq!(params.get_offset(), 40);
    }

    #[test]
    fn pagination_respects_allowed_sort_fields() {
        let params = params(None, None, Some("username"), Some("asc"));
        assert_eq!(
            params.get_order_sql(&["username", "created_at"]),
            "username ASC"
        );
    }

    #[test]
    fn pagination_falls_back_for_unknown_sort_field() {
        let params = params(None, None, Some("password"), Some("DROP TABLE users"));
        assert_eq!(
            params.get_order_sql(&["username", "created_at"]),
            "created_at DESC"
        );
    }

    #[test]
    fn paginated_response_calculates_total_pages() {
        let empty = PaginatedResponse::new(Vec::<i32>::new(), 0, 1, 10);
        assert_eq!(empty.total_pages, 1);

        let page = PaginatedResponse::new(vec![1, 2, 3], 10, 2, 3);
        assert_eq!(page.total_pages, 4);
        assert_eq!(page.page, 2);
        assert_eq!(page.page_size, 3);
    }

    #[test]
    fn paginated_response_guards_zero_page_size() {
        let page = PaginatedResponse::new(vec![1], 1, 0, 0);
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 1);
        assert_eq!(page.total_pages, 1);
    }
}
