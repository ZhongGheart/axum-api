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
use crate::middleware::permission::{
    ensure_can_grant_roles, PermUserCreate, PermUserDelete, PermUserList, PermUserUpdate,
};
use crate::model::{normalize_role_name, ApiResponse, UserInfo, ADMIN_ROLE};
use crate::router::AppState;
use crate::utils::validation;

/// 用户列表查询参数
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UserListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 创建/更新用户请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UserManageRequest {
    pub username: String,
    pub email: String,
    pub password: Option<String>,
    /// 单角色兼容字段（v0.6.0 起 deprecated）
    ///
    /// 保留是为了不破坏既有客户端；新代码请用 [`Self::roles`]。
    /// 两者都给时以 `roles` 为准。
    #[deprecated(note = "角色是多值的，请改用 roles")]
    pub role: Option<String>,
    /// 权威字段：该用户应持有的**全部**角色
    ///
    /// v0.6.0 之前只有单数的 `role`，而数据模型（`user_roles` 表与
    /// `UserInfo.roles`）一直是多角色的：单数字段提交上来会被
    /// `replace_user_roles` **整体替换**成那一个角色，
    /// 用户原有的其余角色被无声删除。
    pub roles: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

/// 用户列表响应
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct UserListResponse {
    pub items: Vec<UserInfo>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

/// 校验并归一化表单提交的角色集合
///
/// 判定"可分配"的依据是**该角色在 `roles` 表里真实存在**，而不是某个常量。
/// 因此 `roles` 表新增一个角色，用户表单立刻就能分配它，无需再改代码。
///
/// v0.6.0：单角色 → 多角色。三个要点：
///
/// - **去重但保持顺序**：表单多选可能重复提交同一角色，
///   `user_roles` 的唯一约束是 `(user_id, role_id)`，重复项会被
///   `ON CONFLICT DO NOTHING` 静默吞掉——留着无害但没必要
/// - **要求非空**：零角色的用户登录后没有任何权限码、侧栏也是空的，
///   属于"建得出、没人能用"的死数据。与 v0.5.0 PR-1 修的半成品用户
///   同类，不该由接口放行
/// - **先全部校验再写**：任一角色不存在即整体拒绝，
///   不留下"部分角色已写入"的中间态
async fn resolve_roles(state: &AppState, raw: &[String]) -> Result<Vec<String>, AppError> {
    let mut resolved: Vec<String> = Vec::with_capacity(raw.len());
    for item in raw {
        let name = normalize_role_name(item)?;
        if state
            .auth_service
            .role_repo
            .find_by_name(&name)
            .await?
            .is_none()
        {
            return Err(AppError::BadRequest(format!("角色「{name}」不存在")));
        }
        if !resolved.contains(&name) {
            resolved.push(name);
        }
    }

    if resolved.is_empty() {
        return Err(AppError::BadRequest(
            "至少需要指定一个角色：没有角色的用户登录后没有任何权限".to_string(),
        ));
    }

    Ok(resolved)
}

/// 从请求体取出权威的角色集合
///
/// `roles` 优先；没给才回退到单数的兼容字段 `role`。
/// 两者都没给返回空切片，交给 [`resolve_roles`] 报"至少一个角色"，
/// 而不是在这里另写一套错误文案。
#[allow(deprecated)]
fn requested_roles(req: &UserManageRequest) -> Vec<String> {
    match (&req.roles, &req.role) {
        (Some(roles), _) => roles.clone(),
        (None, Some(role)) => vec![role.clone()],
        (None, None) => Vec::new(),
    }
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
#[utoipa::path(
    get,
    path = "/api/admin/users",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(
        ("page" = Option<i64>, Query, description = "页码（从 1 开始）"),
        ("page_size" = Option<i64>, Query, description = "每页条数（1-200）"),
    ),
    responses((status = 200, description = "用户列表（含角色）", body = ApiResponse<UserListResponse>))
)]
pub async fn list_users(
    State(state): State<AppState>,
    _perm: PermUserList,
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
#[utoipa::path(
    post,
    path = "/api/admin/users",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    request_body = UserManageRequest,
    responses(
        (status = 200, description = "创建成功", body = ApiResponse<UserInfo>),
        (status = 400, description = "参数不合法"),
        (status = 409, description = "用户名或邮箱已存在"),
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    perm: PermUserCreate,
    Json(req): Json<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    use crate::utils::password::hash_password;

    validation::validate_username(&req.username)?;
    validation::validate_email(&req.email)?;
    // 角色存在性校验必须在写用户之前：仓库层的校验在 replace_user_roles 里，
    // 那时用户行已经落库且两者不在同一事务，失败会留下"没有任何角色的用户"
    let roles = resolve_roles(&state, &requested_roles(&req)).await?;
    // 授权下界（v0.5.0 PR-3）：能创建用户 ≠ 能创建管理员。
    // 闸门撤掉后 `system:user:create` 只说明"可以建号"，建出来的号带什么角色
    // 仍要按"你只能授予自己已持有的权限码"判定。
    ensure_can_grant_roles(
        &state,
        perm.guard(),
        &roles,
        &format!("创建用户并赋予角色 {}", roles.join("、")),
    )
    .await?;

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
    state
        .auth_service
        .role_repo
        .replace_user_roles(user.id, &roles)
        .await?;

    tracing::info!(
        "管理员创建用户: {} (角色: {})",
        user.username,
        roles.join("、")
    );
    Ok(Json(ApiResponse::success(UserInfo::new(user, roles))))
}

/// PUT /api/admin/users/:id — 更新用户（含主角色）
#[utoipa::path(
    put,
    path = "/api/admin/users/{id}",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    request_body = UserManageRequest,
    responses(
        (status = 200, description = "更新成功", body = ApiResponse<UserInfo>),
        (status = 400, description = "参数不合法或试图移除最后一名管理员"),
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    perm: PermUserUpdate,
    Path(id): Path<Uuid>,
    Json(req): Json<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    validation::validate_username(&req.username)?;
    validation::validate_email(&req.email)?;
    let new_roles = resolve_roles(&state, &requested_roles(&req)).await?;

    let current_roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(id)
        .await?;

    // 授权下界（v0.5.0 PR-3）：两道都要查，缺一不可。
    // ① 目标用户当前的权限不得高于自己——否则可以把 admin 降级成普通用户，
    //    借此绕过"最后一名管理员"保护之外的管理边界（改别人权限 = 越权）。
    ensure_can_grant_roles(&state, perm.guard(), &current_roles, "修改该用户").await?;
    // ② 新角色不得高于自己——否则 `role=admin` 就是自我提权。
    ensure_can_grant_roles(
        &state,
        perm.guard(),
        &new_roles,
        &format!("赋予角色 {}", new_roles.join("、")),
    )
    .await?;

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

    tracing::info!(
        "管理员更新用户: {} (角色: {})",
        updated.username,
        new_roles.join("、")
    );
    Ok(Json(ApiResponse::success(UserInfo::new(
        updated, new_roles,
    ))))
}

/// DELETE /api/admin/users/:id — 删除用户
#[utoipa::path(
    delete,
    path = "/api/admin/users/{id}",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    responses(
        (status = 200, description = "删除成功", body = ApiResponse<String>),
        (status = 400, description = "不能删除当前账号或最后一名管理员"),
    )
)]
pub async fn delete_user(
    State(state): State<AppState>,
    perm: PermUserDelete,
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
    // 授权下界（v0.5.0 PR-3）：`system:user:delete` 不该等于"能删 admin"。
    // 注意这条与下面的"最后一名管理员"是两回事：那条防的是把系统锁死，
    // 这条防的是权限高于自己的账号被越权删除。
    ensure_can_grant_roles(&state, perm.guard(), &roles, "删除该用户").await?;
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
#[utoipa::path(
    post,
    path = "/api/admin/users/batch-delete",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    request_body = BatchDeleteRequest,
    responses(
        (status = 200, description = "批量删除成功", body = ApiResponse<String>),
        (status = 400, description = "包含当前账号或会删除全部管理员"),
    )
)]
pub async fn batch_delete_users(
    State(state): State<AppState>,
    perm: PermUserDelete,
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
        // 授权下界（v0.5.0 PR-3）：整批先校验，任一目标权限高于自己就整批拒绝
        ensure_can_grant_roles(&state, perm.guard(), &roles, "批量删除中的用户").await?;
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BatchDeleteRequest {
    pub ids: Vec<Uuid>,
}

/// PUT /api/admin/users/:id/status — 切换状态
#[utoipa::path(
    put,
    path = "/api/admin/users/{id}/status",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    request_body = ToggleStatusRequest,
    responses(
        (status = 200, description = "状态已更新（停用会吊销该用户全部会话）", body = ApiResponse<UserInfo>),
        (status = 400, description = "不能停用当前登录账号"),
    )
)]
pub async fn toggle_user_status(
    State(state): State<AppState>,
    perm: PermUserUpdate,
    auth_user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ToggleStatusRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    if id == auth_user.user_id && !req.is_active {
        return Err(AppError::BadRequest("不能停用当前登录账号".to_string()));
    }

    let user = state.auth_service.user_repo.find_by_id(id).await?;
    // 授权下界（v0.5.0 PR-3）：停用一个权限高于自己的账号 = 拒绝其服务，
    // 与删除同级，属于越权。仅在停用时校验，启用是放宽而非收紧。
    if !req.is_active {
        let current_roles = state
            .auth_service
            .role_repo
            .find_roles_by_user_id(id)
            .await?;
        ensure_can_grant_roles(&state, perm.guard(), &current_roles, "停用该用户").await?;
    }

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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ToggleStatusRequest {
    pub is_active: bool,
}

/// POST /api/admin/users/:id/reset-password — 重置密码
#[utoipa::path(
    post,
    path = "/api/admin/users/{id}/reset-password",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "密码重置成功并吊销该用户全部会话", body = ApiResponse<String>),
        (status = 400, description = "新密码不合法"),
    )
)]
pub async fn reset_user_password(
    State(state): State<AppState>,
    perm: PermUserUpdate,
    Path(id): Path<Uuid>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    use crate::utils::password::hash_password;

    // 先确认用户存在（不存在则 404），再校验新口令
    let user = state.auth_service.user_repo.find_by_id(id).await?;
    // 授权下界（v0.5.0 PR-3）：**这条是所有写路径里最直接的一条**——
    // 拿到 admin 的新口令就等于登录成 admin，不需要再走"授予角色"那一步。
    // 因此 `system:user:update` 单独不足以重置高权限账号的口令。
    let target_roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(id)
        .await?;
    ensure_can_grant_roles(&state, perm.guard(), &target_roles, "重置该用户口令").await?;

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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ResetPasswordRequest {
    pub password: String,
}
