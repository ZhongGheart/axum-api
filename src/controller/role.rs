//! 角色管理控制器
//!
//! 提供角色列表查询、为用户分配/移除角色等接口。

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::permission::{
    codes_of_roles, ensure_can_grant_roles, PermRoleCreate, PermRoleDelete, PermRoleList,
    PermRoleUpdate, PermUserList, PermUserUpdate,
};
use crate::model::{normalize_role_name, ApiResponse, BUILTIN_ROLES};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};
use crate::utils::audit;
use crate::utils::pagination::PaginatedResponse;

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

/// GET /api/admin/roles 的查询参数
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
// utoipa 的 `axum_extras` 本该从 handler 参数推断 `parameter_in`，
// 但 `list_roles` 显式接住拒绝，签名是 `Result<Query<RoleListParams>, QueryRejection>`
// 而不是裸 `Query<...>`，推断不出来 → 回落到 `ParameterIn::default()`，
// 而那个默认值是 **Path**（见 utoipa `openapi/path.rs` 的 `impl Default for ParameterIn`）。
// 结果文档里 `page`/`page_size` 被标成必填**路径**参数，可路径模板里根本没有 `{page}`——
// Swagger UI 会把它们渲染成路径输入框，按规范校验也是无效文档。
// 显式钉死 Query，才不依赖这个默认值。
#[into_params(parameter_in = Query)]
pub struct RoleListParams {
    /// 页码（从 1 开始）
    pub page: Option<i64>,
    /// 每页条数（1-200）
    pub page_size: Option<i64>,
}

/// 分配角色请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AssignRoleRequest {
    /// 与路径参数 `user_id` 重复，服务端一律以**路径**为准。
    ///
    /// 刻意是可选的：它曾是必填，于是漏传时 `Json` 提取器先失败、返回 400，
    /// 无权限调用者因此拿到了本不该看到的入参结构反馈——正是
    /// `middleware::permission` 开头那条"鉴权必须早于入参校验"要避免的事。
    /// 保留字段只为兼容既有客户端（含前端 `api/role.ts`）。
    #[allow(dead_code)]
    pub user_id: Option<Uuid>,
    pub role_name: String,
}

/// GET /api/admin/roles — 角色列表（含用户数，分页）
#[utoipa::path(
    get,
    path = "/api/admin/roles",
    tag = "角色管理",
    security(("bearer_auth" = [])),
    params(RoleListParams),
    responses((status = 200, description = "角色列表（含关联用户数）", body = ApiResponse<PaginatedResponse<RoleItem>>))
)]
pub async fn list_roles(
    State(state): State<AppState>,
    _perm: PermRoleList,
    params: Result<Query<RoleListParams>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<ApiResponse<PaginatedResponse<RoleItem>>>, AppError> {
    // 显式接住拒绝，错误才走统一响应格式（见 `From<QueryRejection>`）
    let Query(params) = params?;
    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(10);
    crate::utils::validation::validate_page(page, page_size)?;

    let (rows, total) = state
        .auth_service
        .role_repo
        .list_paginated(page, page_size)
        .await?;
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

    Ok(Json(ApiResponse::success(PaginatedResponse::new(
        items, total, page, page_size,
    ))))
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
    ApiPath(user_id): ApiPath<Uuid>,
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
    perm: PermUserUpdate,
    audit: AuditDetail,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<AssignRoleRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 与用户表单的 role 字段走同一套归一化，否则同一个角色
    // 经本接口提交 "Admin" 会 404、经表单提交 "admin" 却成功
    let role_name = normalize_role_name(&req.role_name)?;
    // 目标用户必须存在。此前本接口直接写 `user_roles`，用户不存在时
    // 外键违例冒成 500「服务器内部错误」——与 v0.8.0 修的
    // 「声明已占用的权限码冒成 500」同源：入参错误被当成服务端故障，
    // 既污染错误监控，调用方也看不懂到底是路径错了还是系统坏了。
    let target_user = state.auth_service.user_repo.find_by_id(user_id).await?;
    let current_roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(user_id)
        .await?;
    // 授权下界（v0.5.0 PR-3）：本接口是**追加**语义，不会覆盖既有角色，
    // 因此用户表单那道"整体替换"的守卫覆盖不到它——`role_name=admin`
    // 曾经是一条独立的提权路径，必须单独判定。
    ensure_can_grant_roles(
        &state,
        perm.guard(),
        std::slice::from_ref(&role_name),
        &format!("追加角色「{role_name}」"),
    )
    .await?;
    // 授权下界（v0.9.0）：还要查**目标用户当前的角色**。
    // 上一条只管"授予什么"，不管"授予给谁"，于是只持 `system:user:update`
    // 的角色能给一个纯 admin 账号追加角色（实测 200，角色真的变了），
    // 而 `update_user` / `delete_user` / `batch_delete_users` 三处都查目标。
    // 把自己追加弱角色不受影响：调用者天然覆盖自己的全部权限码，
    // 这条守卫对自己是恒真的——合法的自我降级路径不会被误伤。
    ensure_can_grant_roles(&state, perm.guard(), &current_roles, "修改该用户的角色").await?;
    // 幂等：已持有该角色时不写库、也不吊销会话。
    // `assign_role_to_user` 用 `ON CONFLICT DO NOTHING`，重复调用本就什么都没写，
    // 若照样吊销会话就是把一次无操作变成一次强制登出。
    let already_held = current_roles.iter().any(|r| r == &role_name);
    state
        .auth_service
        .role_repo
        .assign_role_to_user(user_id, &role_name)
        .await?;
    if !already_held {
        // 权限已变化：吊销存量会话，旧令牌不得继续携带旧角色。
        // 与 `update_user` 同一套处理——否则新授的码要等目标用户
        // 自己重新登录才生效（实测：追加后原令牌仍 403，重新登录才 200）。
        state
            .auth_service
            .revoke_all_sessions(&state.redis_client, user_id)
            .await?;
        // 已持有时不写：重复追加什么都没发生，记成"已授予"是假阳性
        audit.push(format!(
            "为用户 \"{}\" 追加角色 \"{role_name}\"（{user_id}）",
            target_user.username
        ));
    }
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
    audit: AuditDetail,
    ApiJson(req): ApiJson<CreateRoleReq>,
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
    audit.push(format!("新建角色 \"{name}\"（{id}）"));
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CreateRoleReq>,
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

    // 改名要**两个名字都记**：事后只看到新名字，仍然答不出
    // "这个角色原来叫什么"——而角色名是 user_roles 之外唯一的人类可读标识
    if row.1 != current_name {
        audit.push(format!(
            "角色 \"{current_name}\" 改名为 \"{}\"（{id}）",
            row.1
        ));
    } else {
        audit.push(format!("更新角色 \"{}\"（{id}）", row.1));
    }

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
    perm: PermRoleDelete,
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
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

    // 授权下界（v0.9.0）：删角色 = 把这个角色承载的权限码从所有人身上撤走，
    // 与 `PUT /roles/:id/menus`（给角色授权）是同一件事的两面，那条路 v0.7.0
    // 就装了这道天花板，这条路当时没管。
    //
    // 实测（探针，非读代码推断）：只持 `system:role:delete` 的操作员可以删掉
    // 一个承载 `system:log:list` 的角色——那个码他自己并不持有。
    // 于是"只能授予自己已持有的权限"这条不变量，在删除这条路上是失效的。
    //
    // 放在内置角色检查**之后**：内置角色永不可删，这是与权限无关的固有事实，
    // 若先报"缺少权限：X"，操作员会误以为拿到 X 就能删内置角色——
    // 那是在把人往错误的方向引。鉴权该早于的是"这个角色有几个人在用"
    // 这类**随调用者而变**的信息，它仍在下面那句校验之前。
    //
    // 不像 `delete_menu` 那样要判断"是否有人依赖"——角色只要存在，
    // 它携带的码就都在生效，删掉必然改变每个人的权限。
    let granted_codes = codes_of_roles(&state, std::slice::from_ref(&name)).await?;
    if !granted_codes.is_empty() {
        perm.guard().ensure_covers(
            &granted_codes,
            &format!("删除承载权限码「{}」的角色", granted_codes.join("、")),
        )?;
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
    // 名字只在 `roles` 行里，删掉就永久没有了（`role_menus` 已被外键级联清掉）
    let revoked = audit::codes("随之撤销的权限码", &granted_codes);
    audit.push(match revoked.is_empty() {
        true => format!("删除角色 \"{name}\"（{id}）"),
        false => format!("删除角色 \"{name}\"（{id}）；{revoked}"),
    });
    Ok(Json(ApiResponse::success("角色删除成功")))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateRoleReq {
    pub name: String,
    pub description: Option<String>,
}
