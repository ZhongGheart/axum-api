//! 操作日志模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 操作日志数据库实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AuditLog {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub username: Option<String>,
    pub action: String,
    pub method: String,
    pub path: String,
    pub params: Option<String>,
    pub result: Option<String>,
    pub status_code: Option<i32>,
    pub client_ip: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}
