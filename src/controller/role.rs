//! 角色管理控制器
//!
//! 提供角色列表查询、为用户分配/移除角色等接口。

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::permission::{
    PermRoleCreate, PermRoleDelete, PermRoleList, PermRoleUpdate, PermUserList, PermUserUpdate,
};
use crate::model::{normalize_role_name, ApiResponse, BUILTIN_ROLES};
use crate::router::AppState;

/// `roles.name` 的唯一约束名（`name VARCHAR(50) NOT NULL UNIQUE`）
const ROLES_NAME_KEY: &str = "roles_name_key";

/// 角色列表项
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RoleItem {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub user_count: i64,
}

/// 分配角色请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AssignRoleRequest {
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub role_name: String,
}

/// GET /api/admin/roles — 角色列表（含用户数）
#[utoipa::path(
    get,
    path = "/api/admin/roles",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "角色列表（含关联用户数）", body = ApiResponse<Vec<RoleItem>>))
)]
pub async fn list_roles(
    State(state): State<AppState>,
    _perm: PermRoleList,
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
#[utoipa::path(
    get,
    path = "/api/admin/users/{user_id}/roles",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    params(("user_id" = Uuid, Path, description = "用户 ID")),
    responses((status = 200, description = "用户当前角色名列表", body = ApiResponse<Vec<String>>))
)]
pub async fn get_user_roles(
    State(state): State<AppState>,
    _perm: PermUserList,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(user_id)
        .await?;
    Ok(Json(ApiResponse::success(roles)))
}

/// POST /api/admin/users/:id/roles — 为用户分配角色
#[utoipa::path(
    post,
    path = "/api/admin/users/{user_id}/roles",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    params(("user_id" = Uuid, Path, description = "用户 ID")),
    request_body = AssignRoleRequest,
    responses(
        (status = 200, description = "角色已追加", body = ApiResponse<String>),
        (status = 400, description = "角色名不合法"),
        (status = 404, description = "角色不存在"),
    )
)]
pub async fn assign_user_role(
    State(state): State<AppState>,
    _perm: PermUserUpdate,
    axum::extract::Path(user_id): axum::extract::Path<Uuid>,
    Json(req): Json<AssignRoleRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 与用户表单的 role 字段走同一套归一化，否则同一个角色
    // 经本接口提交 "Admin" 会 404、经表单提交 "admin" 却成功
    let role_name = normalize_role_name(&req.role_name)?;
    state
        .auth_service
        .role_repo
        .assign_role_to_user(user_id, &role_name)
        .await?;
    Ok(Json(ApiResponse::success("角色分配成功")))
}

/// POST /api/admin/roles — 新增角色
#[utoipa::path(
    post,
    path = "/api/admin/roles",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    request_body = CreateRoleReq,
    responses(
        (status = 200, description = "创建成功", body = ApiResponse<RoleItem>),
        (status = 400, description = "角色名不合法"),
        (status = 409, description = "角色名已被占用"),
    )
)]
pub async fn create_role(
    State(state): State<AppState>,
    _perm: PermRoleCreate,
    Json(req): Json<CreateRoleReq>,
) -> Result<Json<ApiResponse<RoleItem>>, AppError> {
    let name = normalize_role_name(&req.name)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO roles (id, name, description) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&name)
        .bind(&req.description)
        .execute(&state.auth_service.user_repo.pool)
        .await
        .map_err(|e| {
            // 归一化后撞名是常见操作（建 Auditor 再建 auditor），不该吐 500
            if let Some(pg_err) = e.as_database_error() {
                if pg_err.constraint() == Some(ROLES_NAME_KEY) {
                    return AppError::Conflict(format!("角色名「{name}」已被占用"));
                }
            }
            AppError::InternalServerError(format!("创建角色失败: {e}"))
        })?;
    Ok(Json(ApiResponse::success(RoleItem {
        id,
        name,
        description: req.description,
        created_at: chrono::Utc::now(),
        user_count: 0,
    })))
}

/// PUT /api/admin/roles/:id — 更新角色
#[utoipa::path(
    put,
    path = "/api/admin/roles/{id}",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "角色 ID")),
    request_body = CreateRoleReq,
    responses(
        (status = 200, description = "更新成功", body = ApiResponse<RoleItem>),
        (status = 400, description = "角色名不合法，或试图改名内置角色"),
        (status = 404, description = "角色不存在"),
        (status = 409, description = "角色名已被占用"),
    )
)]
pub async fn update_role(
    State(state): State<AppState>,
    _perm: PermRoleUpdate,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<CreateRoleReq>,
) -> Result<Json<ApiResponse<RoleItem>>, AppError> {
    let name = normalize_role_name(&req.name)?;

    let mut tx = state
        .auth_service
        .user_repo
        .pool
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

    // FOR UPDATE：并发改名时锁住这一行，避免两个请求都读到旧名后双双通过校验
    let current: Option<(String,)> =
        sqlx::query_as("SELECT name FROM roles WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;
    let Some((current_name,)) = current else {
        return Err(AppError::NotFound("角色不存在".into()));
    };

    // 改名内置角色与删除内置角色是同一类破坏：`ADMIN_ROLE = "admin"`
    // 是最后一名管理员保护和权限码种子的查找依据，改名后这些依据全部落空
    if BUILTIN_ROLES.contains(&current_name.as_str()) {
        return Err(AppError::BadRequest(format!(
            "内置角色「{current_name}」不可改名"
        )));
    }
    if BUILTIN_ROLES.contains(&name.as_str()) {
        return Err(AppError::BadRequest(format!(
            "不能把角色改名为内置角色名「{name}」"
        )));
    }

    sqlx::query("UPDATE roles SET name = $1, description = $2 WHERE id = $3")
        .bind(&name)
        .bind(&req.description)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if let Some(pg_err) = e.as_database_error() {
                if pg_err.constraint() == Some(ROLES_NAME_KEY) {
                    return AppError::Conflict(format!("角色名「{name}」已被占用"));
                }
            }
            AppError::InternalServerError(format!("更新角色失败: {e}"))
        })?;

    // 回读真实行：原先这里编造 created_at = now()、user_count = 0，
    // 更新一个 50 人角色也会回一个"0 人、刚创建"的角色
    let row: (
        Uuid,
        String,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        i64,
    ) = sqlx::query_as(
        r#"
        SELECT r.id, r.name, r.description, r.created_at,
               COALESCE(ur_cnt.cnt, 0) AS user_count
        FROM roles r
        LEFT JOIN (SELECT role_id, COUNT(*) AS cnt FROM user_roles GROUP BY role_id) ur_cnt
          ON ur_cnt.role_id = r.id
        WHERE r.id = $1
        "#,
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| AppError::InternalServerError(format!("回读角色失败: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;

    Ok(Json(ApiResponse::success(RoleItem {
        id: row.0,
        name: row.1,
        description: row.2,
        created_at: row.3,
        user_count: row.4,
    })))
}

/// DELETE /api/admin/roles/:id — 删除角色
#[utoipa::path(
    delete,
    path = "/api/admin/roles/{id}",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "角色 ID")),
    responses((status = 200, description = "删除成功", body = ApiResponse<String>))
)]
pub async fn delete_role(
    State(state): State<AppState>,
    _perm: PermRoleDelete,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    let mut tx = state
        .auth_service
        .user_repo
        .pool
        .begin()
        .await
        .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

    // FOR UPDATE：并发删除时先锁住这一行，避免两个请求同时通过下面的校验
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM roles WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;
    let Some((name,)) = name else {
        return Err(AppError::NotFound("角色不存在".into()));
    };

    // 内置角色不可删除：角色种子只在 roles 表为空时写入，删掉不会被重建
    if BUILTIN_ROLES.contains(&name.as_str()) {
        return Err(AppError::BadRequest(format!("内置角色「{name}」不可删除")));
    }

    // 仍有用户持有该角色时拒绝删除：user_roles 的 ON DELETE CASCADE 会
    // 静默剥掉这些用户的角色，让人变成"没有任何角色"的用户而不自知
    let (user_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM user_roles WHERE role_id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("统计角色用户数失败: {e}")))?;
    if user_count > 0 {
        return Err(AppError::BadRequest(format!(
            "仍有 {user_count} 个用户使用该角色，请先调整这些用户的角色"
        )));
    }

    // role_menus 由 role_id 外键的 ON DELETE CASCADE 一并清理，无需手工删除
    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("删除角色失败: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;
    Ok(Json(ApiResponse::success("角色删除成功")))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateRoleReq {
    pub name: String,
    pub description: Option<String>,
}
