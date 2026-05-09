//! 通用分页查询封装
//!
//! 标准化分页参数、排序、多条件筛选。
//! 前端和后端统一使用此参数结构。

use serde::{Deserialize, Serialize};

/// 统一分页请求参数
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PaginationParams {
    /// 页码（从 1 开始）
    pub page: Option<i64>,
    /// 每页条数
    pub page_size: Option<i64>,
    /// 排序字段
    pub sort_by: Option<String>,
    /// 排序方向（asc / desc）
    pub sort_order: Option<String>,
    /// 关键字搜索
    pub keyword: Option<String>,
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

    /// 生成过滤条件（LIKE 模糊搜索）
    pub fn keyword_like(&self) -> String {
        self.keyword.as_deref().unwrap_or("").to_string()
    }
}

/// 统一分页响应数据
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        let total_pages = if total == 0 { 1 } else { (total as f64 / page_size as f64).ceil() as i64 };
        Self { items, total, page, page_size, total_pages }
    }
}

/// 前端对齐的分页请求体（与前端 PageParams 一致）
#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}
