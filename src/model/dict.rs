//! 数据字典模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 字典类型实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DictType {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 字典项实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DictItem {
    pub id: Uuid,
    pub dict_type_id: Uuid,
    pub label: String,
    pub value: String,
    pub sort_order: i32,
    pub status: String,
    pub is_default: bool,
    pub color: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 字典类型 + 项（给前端一次性返回）
#[derive(Debug, Serialize, Deserialize)]
pub struct DictTypeWithItems {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub sort_order: i32,
    pub items: Vec<DictItemResponse>,
}

/// 字典项响应
#[derive(Debug, Serialize, Deserialize)]
pub struct DictItemResponse {
    pub id: Uuid,
    pub label: String,
    pub value: String,
    pub sort_order: i32,
    pub status: String,
    pub is_default: bool,
    pub color: Option<String>,
}

/// 字典类型请求
#[derive(Debug, Deserialize)]
pub struct CreateDictTypeRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub sort_order: Option<i32>,
}

/// 字典项请求
#[derive(Debug, Deserialize)]
pub struct CreateDictItemRequest {
    pub dict_type_id: Option<Uuid>,
    pub label: String,
    pub value: String,
    pub sort_order: Option<i32>,
    pub status: Option<String>,
    pub is_default: Option<bool>,
    pub color: Option<String>,
}
