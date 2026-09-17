//! 用户管理控制器
//!
//! 提供管理员操作用户的接口：列表、创建、更新、删除、状态切换、重置密码。
//!
//! 角色一致性约定：
//! - 角色的唯一数据源是 `user_roles` 表（`users.role` 列已在 v0.2 移除）
//! - 用户表单里的 `role` 字段表示"主角色"，落地为整体替换角色集合
//! - 角色或状态发生变化时吊销该用户存量会话，避免旧令牌继续携带旧权限

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::model::{ApiResponse, UserInfo};
use crate::router::AppState;
use crate::utils::validation;

/// 允许通过用户表单分配的内置角色
const ASSIGNABLE_ROLES: [&str; 2] = ["admin", "user"];
/// 管理员角色标识
const ADMIN_ROLE: &str = "admin";

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

/// 校验并归一化表单角色名
fn normalize_role(role: &str) -> Result<String, AppError> {
    let role = role.trim().to_lowercase();
    if !ASSIGNABLE_ROLES.contains(&role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "角色必须是 {} 之一",
            ASSIGNABLE_ROLES.join(" / ")
        )));
    }
    Ok(role)
}

/// 批量构建 user_id → 角色列表 映射（避免列表页 N+1 查询）
async fn roles_map(state: &AppState, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<String>>, AppError> {
    let mut map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (user_id, role) in state
        .auth_service
        .user_repo
        .find_roles_for_users(ids)
        .await?
    {
        map.entry(user_id).or_default().push(role);
    }
    Ok(map)
}

/// 阻止"最后一名管理员"失去 admin 角色
async fn ensure_not_last_admin(
    state: &AppState,
    current_roles: &[String],
    new_roles: &[String],
) -> Result<(), AppError> {
    let was_admin = current_roles.iter().any(|r| r == ADMIN_ROLE);
    let stays_admin = new_roles.iter().any(|r| r == ADMIN_ROLE);
    if was_admin && !stays_admin {
        let admins = state
            .auth_service
            .role_repo
            .count_users_with_role(ADMIN_ROLE)
            .await?;
        if admins <= 1 {
            return Err(AppError::BadRequest(
                "不能移除最后一名管理员的 admin 角色".to_string(),
            ));
        }
    }
    Ok(())
}

/// 判断两个角色集合是否等价（顺序无关）
fn same_role_set(a: &[String], b: &[String]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

/// GET /api/admin/users — 用户列表（分页）
pub async fn list_users(
    State(state): State<AppState>,
    Query(params): Query<UserListParams>,
) -> Result<Json<ApiResponse<UserListResponse>>, AppError> {
    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(10);
    validation::validate_page(page, page_size)?;

    let (users, total) = state
        .auth_service
        .user_repo
        .list_all(page, page_size)
        .await?;

    let ids: Vec<Uuid> = users.iter().map(|u| u.id).collect();
    let mut grouped = roles_map(&state, &ids).await?;

    // 列表一次性补齐角色，避免 UserInfo.roles 恒为空
    let items: Vec<UserInfo> = users
        .into_iter()
        .map(|u| {
            let roles = grouped.remove(&u.id).unwrap_or_default();
            UserInfo::new(u, roles)
        })
        .collect();

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

    validation::validate_username(&req.username)?;
    validation::validate_email(&req.email)?;
    let role = normalize_role(&req.role)?;

    let password = req.password.as_deref().unwrap_or("password123");
    validation::validate_password(password)?;

    if state
        .auth_service
        .user_repo
        .find_by_username(&req.username)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("用户名已被占用".to_string()));
    }
    if state
        .auth_service
        .user_repo
        .find_by_email(&req.email)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("邮箱已被占用".to_string()));
    }

    let password_hash =
        hash_password(password).map_err(|e| AppError::InternalServerError(e.to_string()))?;

    let user = state
        .auth_service
        .user_repo
        .create(Uuid::new_v4(), &req.username, &req.email, &password_hash)
        .await?;

    // 角色写入 user_roles（唯一数据源），事务内完成
    let assigned = vec![role.clone()];
    state
        .auth_service
        .role_repo
        .replace_user_roles(user.id, &assigned)
        .await?;

    tracing::info!("管理员创建用户: {} (角色: {role})", user.username);
    Ok(Json(ApiResponse::success(UserInfo::new(user, assigned))))
}

/// PUT /api/admin/users/:id — 更新用户（含主角色）
pub async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    validation::validate_username(&req.username)?;
    validation::validate_email(&req.email)?;
    let role = normalize_role(&req.role)?;

    let current_roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(id)
        .await?;
    let new_roles = vec![role.clone()];

    // 先做守卫，避免"基础字段已更新但角色变更被拒绝"的半成品状态
    ensure_not_last_admin(&state, &current_roles, &new_roles).await?;

    let updated = state
        .auth_service
        .user_repo
        .update(id, &req.username, &req.email, req.is_active.unwrap_or(true))
        .await?;

    if !same_role_set(&current_roles, &new_roles) {
        state
            .auth_service
            .role_repo
            .replace_user_roles(id, &new_roles)
            .await?;
        // 权限已变化：吊销存量会话，旧令牌不得继续携带旧角色
        state
            .auth_service
            .revoke_all_sessions(&state.redis_client, id)
            .await?;
    }

    tracing::info!("管理员更新用户: {} (角色: {role})", updated.username);
    Ok(Json(ApiResponse::success(UserInfo::new(
        updated, new_roles,
    ))))
}

/// DELETE /api/admin/users/:id — 删除用户
pub async fn delete_user(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    if id == auth_user.user_id {
        return Err(AppError::BadRequest("不能删除当前登录账号".to_string()));
    }

    let user = state.auth_service.user_repo.find_by_id(id).await?;
    let roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(id)
        .await?;
    ensure_not_last_admin(&state, &roles, &[]).await?;

    // user_roles 通过外键 ON DELETE CASCADE 一并清理
    state.auth_service.user_repo.delete(id).await?;
    state
        .auth_service
        .revoke_all_sessions(&state.redis_client, id)
        .await?;

    tracing::info!("管理员删除用户: {} ({})", user.username, id);
    Ok(Json(ApiResponse::success("删除成功")))
}

/// POST /api/admin/users/batch-delete — 批量删除
pub async fn batch_delete_users(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(req): Json<BatchDeleteRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    if req.ids.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个用户".to_string()));
    }
    if req.ids.contains(&auth_user.user_id) {
        return Err(AppError::BadRequest("不能删除当前登录账号".to_string()));
    }

    // 先整体校验：逐个删除时无法发现"这一批会删掉全部管理员"
    let mut admins_in_batch = 0i64;
    for id in &req.ids {
        state.auth_service.user_repo.find_by_id(*id).await?;
        let roles = state
            .auth_service
            .role_repo
            .find_roles_by_user_id(*id)
            .await?;
        if roles.iter().any(|r| r == ADMIN_ROLE) {
            admins_in_batch += 1;
        }
    }

    if admins_in_batch > 0 {
        let total_admins = state
            .auth_service
            .role_repo
            .count_users_with_role(ADMIN_ROLE)
            .await?;
        if total_admins - admins_in_batch < 1 {
            return Err(AppError::BadRequest(
                "不能删除全部管理员，系统至少需要保留一名管理员".to_string(),
            ));
        }
    }

    for id in &req.ids {
        state.auth_service.user_repo.delete(*id).await?;
        state
            .auth_service
            .revoke_all_sessions(&state.redis_client, *id)
            .await?;
    }

    tracing::info!("管理员批量删除用户: {} 个", req.ids.len());
    Ok(Json(ApiResponse::success("批量删除成功")))
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteRequest {
    pub ids: Vec<Uuid>,
}

/// PUT /api/admin/users/:id/status — 切换状态
pub async fn toggle_user_status(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ToggleStatusRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    if id == auth_user.user_id && !req.is_active {
        return Err(AppError::BadRequest("不能停用当前登录账号".to_string()));
    }

    let user = state.auth_service.user_repo.find_by_id(id).await?;
    let updated = state
        .auth_service
        .user_repo
        .update(id, &user.username, &user.email, req.is_active)
        .await?;

    // 停用账号必须立即失效其全部会话
    if !req.is_active {
        state
            .auth_service
            .revoke_all_sessions(&state.redis_client, id)
            .await?;
    }

    let roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(id)
        .await?;
    Ok(Json(ApiResponse::success(UserInfo::new(updated, roles))))
}

#[derive(Debug, Deserialize)]
pub struct ToggleStatusRequest {
    pub is_active: bool,
}

/// POST /api/admin/users/:id/reset-password — 重置密码
pub async fn reset_user_password(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    use crate::utils::password::hash_password;

    // 先确认用户存在（不存在则 404），再校验新口令
    let user = state.auth_service.user_repo.find_by_id(id).await?;
    validation::validate_password(&req.password)?;

    let hashed =
        hash_password(&req.password).map_err(|e| AppError::InternalServerError(e.to_string()))?;
    state
        .auth_service
        .user_repo
        .update_password_hash(id, &hashed)
        .await?;

    // 口令已变化：吊销该用户全部存量会话
    state
        .auth_service
        .revoke_all_sessions(&state.redis_client, id)
        .await?;

    tracing::info!("管理员重置用户密码并吊销会话: {}", user.username);
    Ok(Json(ApiResponse::success("密码重置成功")))
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub password: String,
}
