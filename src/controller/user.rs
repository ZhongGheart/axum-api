//! 用户管理控制器
//!
//! 提供管理员操作用户的接口：列表、创建、更新、删除。

use axum::{extract::State, Json};
use serde::Deserialize;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{ApiResponse, UserInfo};
use crate::router::AppState;

/// 用户列表查询参数
#[derive(Debug, Deserialize)]
pub struct UserListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 创建/更新用户请求
#[derive(Debug, Deserialize)]
pub struct UserManageRequest {
    pub username: String,
    pub email: String,
    pub password: Option<String>,
    pub role: String,
    pub is_active: Option<bool>,
}

/// 用户列表响应
#[derive(Debug, serde::Serialize)]
pub struct UserListResponse {
    pub items: Vec<UserInfo>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

/// GET /api/admin/users — 用户列表（分页）
pub async fn list_users(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<UserListParams>,
) -> Result<Json<ApiResponse<UserListResponse>>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(10).clamp(1, 100);

    let (users, total) = state.auth_service.user_repo.list_all(page, page_size).await?;

    let items: Vec<UserInfo> = users.into_iter().map(UserInfo::from).collect();
    let total_pages = (total as f64 / page_size as f64).ceil() as i64;

    Ok(Json(ApiResponse::success(UserListResponse {
        items,
        total,
        page,
        page_size,
        total_pages,
    })))
}

/// POST /api/admin/users — 创建用户（含角色分配）
pub async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    use crate::utils::password::hash_password;

    if req.username.len() < 3 || req.username.len() > 50 {
        return Err(AppError::BadRequest("用户名长度必须在 3-50 个字符之间".to_string()));
    }
    if !req.email.contains('@') {
        return Err(AppError::BadRequest("邮箱格式不正确".to_string()));
    }
    let password = req.password.as_deref().unwrap_or("password123");
    if password.len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于 6 个字符".to_string()));
    }

    let password_hash = hash_password(password)
        .map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let user = state.auth_service.user_repo
        .create(Uuid::new_v4(), &req.username, &req.email, &password_hash)
        .await?;

    // 分配角色
    state.auth_service.role_repo
        .assign_role_to_user(user.id, &req.role)
        .await?;

    tracing::info!("管理员创建用户: {} (角色: {})", user.username, req.role);
    Ok(Json(ApiResponse::success(UserInfo::from(user))))
}

/// PUT /api/admin/users/:id — 更新用户
pub async fn update_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user = state.auth_service.user_repo
        .update(id, &req.username, &req.email, &req.role, req.is_active.unwrap_or(true))
        .await?;

    tracing::info!("管理员更新用户: {}", user.username);
    Ok(Json(ApiResponse::success(UserInfo::from(user))))
}

/// DELETE /api/admin/users/:id — 删除用户
pub async fn delete_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.auth_service.user_repo.delete(id).await?;
    tracing::info!("管理员删除用户: {}", id);
    Ok(Json(ApiResponse::success("删除成功")))
}
