//! 数据字典模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 字典类型实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DictItemResponse {
    pub id: Uuid,
    pub label: String,
    pub value: String,
    pub sort_order: i32,
    pub status: String,
    pub is_default: bool,
    pub color: Option<String>,
}

/// 「刷新字典缓存」的真实结果
///
/// 此前这个端点无条件返回 `"缓存刷新成功"` ——而它实际上一个键都没删。
/// 拆成三个数字，是为了让"成功"这三个字有可核对的内容：
/// 管理员能看出到底清了几条、有几个类型没被回填。
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DictCacheRefresh {
    /// 实际删除的缓存键数
    pub cleared_keys: u64,
    /// 重新载入缓存的字典类型数（仅 `status=enabled` 的）
    pub reloaded_types: u64,
    /// 跳过的已禁用类型数：禁用类型不进读取端点，缓存了也没人读
    pub skipped_disabled_types: u64,
}

/// 字典类型请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDictTypeRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub sort_order: Option<i32>,
}

/// 字典项请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDictItemRequest {
    pub dict_type_id: Option<Uuid>,
    pub label: String,
    pub value: String,
    pub sort_order: Option<i32>,
    pub status: Option<String>,
    pub is_default: Option<bool>,
    pub color: Option<String>,
}
