//! 角色管理控制器
//!
//! 提供角色列表查询、为用户分配/移除角色等接口。

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::ApiResponse;
use crate::router::AppState;

/// 角色列表项
#[derive(Debug, Serialize)]
pub struct RoleItem {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub user_count: i64,
}

/// 分配角色请求
#[derive(Debug, Deserialize)]
pub struct AssignRoleRequest {
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub role_name: String,
}

/// GET /api/admin/roles — 角色列表（含用户数）
pub async fn list_roles(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<RoleItem>>>, AppError> {
    let rows = state.auth_service.role_repo.list_all().await?;
    let items: Vec<RoleItem> = rows
        .into_iter()
        .map(|(role, user_count)| RoleItem {
            id: role.id,
            name: role.name,
            description: role.description,
            created_at: role.created_at,
            user_count,
        })
        .collect();
    Ok(Json(ApiResponse::success(items)))
}

/// GET /api/admin/users/:id/roles — 获取用户已分配的角色
pub async fn get_user_roles(
    State(state): State<AppState>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let roles = state.auth_service.role_repo.find_roles_by_user_id(user_id).await?;
    Ok(Json(ApiResponse::success(roles)))
}

/// POST /api/admin/users/:id/roles — 为用户分配角色
pub async fn assign_user_role(
    State(state): State<AppState>,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
    Json(req): Json<AssignRoleRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.auth_service.role_repo.assign_role_to_user(user_id, &req.role_name).await?;
    Ok(Json(ApiResponse::success("角色分配成功")))
}
