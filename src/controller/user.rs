//! 用户管理控制器
//!
//! 提供管理员操作用户的接口：列表、创建、更新、删除、状态切换、重置密码。
//!
//! 角色一致性约定：
//! - 角色的唯一数据源是 `user_roles` 表（`users.role` 列已在 v0.2 移除）
//! - 用户表单里的 `role` 字段表示"主角色"，落地为整体替换角色集合
//! - 角色或状态发生变化时吊销该用户存量会话，避免旧令牌继续携带旧权限

use std::collections::HashMap;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::permission::{
    ensure_can_grant_roles, PermSessionManage, PermUserCreate, PermUserDelete, PermUserList,
    PermUserUnlock, PermUserUpdate,
};
use crate::model::{normalize_role_name, ApiResponse, UserInfo, ADMIN_ROLE};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};
use crate::utils::audit;
use crate::utils::validation;

/// 用户列表查询参数
///
/// `deny_unknown_fields` 是本版的核心决定之一：前端多传一个字段，
/// 此前会被 `serde` **静默丢弃**——筛选栏摆在那儿、参数也确实发出去了，
/// 后端却当它不存在，于是界面表现为"搜索没反应"。
/// 现在多传直接 400 并指名那个字段，让错误**响着发生**。
///
/// 注意：`deny_unknown_fields` 与 `serde(flatten)` 不兼容，
/// 所以本结构体显式列出全部字段，不能靠 flatten 复用 `PaginationParams`。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UserListParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    /// 关键字，同时匹配用户名与邮箱；空串等同不过滤
    pub keyword: Option<String>,
    /// 激活状态筛选：`true` 只看启用，`false` 只看禁用，不传则不限
    ///
    /// 用 `Option<bool>` 而不是裸 `bool`：裸 bool 无法表达"不限"，
    /// 而默认成"只看启用"会让用户列表凭空少掉所有禁用账号——
    /// 管理员恰恰最需要看见被自己禁掉的那些。
    pub is_active: Option<bool>,
    /// 角色名筛选：只看拥有该角色的用户；空串等同不过滤
    pub role: Option<String>,
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
        ("keyword" = Option<String>, Query, description = "关键字，同时匹配用户名与邮箱"),
        ("is_active" = Option<bool>, Query, description = "按启用状态筛选，不传则不限"),
        ("role" = Option<String>, Query, description = "按角色名筛选，只看拥有该角色的用户"),
    ),
    responses((status = 200, description = "用户列表（含角色）", body = ApiResponse<UserListResponse>))
)]
pub async fn list_users(
    State(state): State<AppState>,
    _perm: PermUserList,
    params: Result<Query<UserListParams>, QueryRejection>,
) -> Result<Json<ApiResponse<UserListResponse>>, AppError> {
    // 显式接住拒绝，错误才走统一响应格式（见 `From<QueryRejection>`）
    let Query(params) = params?;
    let page = params.page.unwrap_or(1);
    let page_size = params.page_size.unwrap_or(10);
    validation::validate_page(page, page_size)?;

    // 三个维度一起交给仓储的筛选结构体，由它统一拼 WHERE 并保证
    // 列表与 COUNT 用同一组条件（漏改一处就是分页错乱，见仓储注释）。
    let filter = crate::repository::user::UserListFilter {
        keyword: params.keyword.as_deref(),
        is_active: params.is_active,
        role_name: params.role.as_deref(),
    };
    let (users, total) = state
        .auth_service
        .user_repo
        .list_filtered(page, page_size, &filter)
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
    audit: AuditDetail,
    ApiJson(req): ApiJson<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    use crate::utils::password::hash_password;

    // 归一在**校验之前**（见 `validation::normalize_username`）：
    // 下面的查重用的就是归一后的值，查重因此自动是"归一后比较"，
    // 不必再单独写一条大小写不敏感的查重——两处规则迟早会走偏。
    let username = validation::normalize_username(&req.username)?;
    let email = validation::normalize_email(&req.email)?;
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
    // 策略来自参数表（v0.22.0），管理员改完立即对新建账号生效
    validation::validate_password_with(password, &state.setting_service.password_policy().await)?;

    if state
        .auth_service
        .user_repo
        .find_by_username(&username)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("用户名已被占用".to_string()));
    }
    if state
        .auth_service
        .user_repo
        .find_by_email(&email)
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
        .create(
            Uuid::new_v4(),
            &username,
            &email,
            &password_hash,
            // 管理员建号：口令由管理员设定，用户本人从未参与选择，
            // 因此强制其首次登录后改掉（v0.11.0）
            true,
        )
        .await?;

    // 角色写入 user_roles（唯一数据源），事务内完成
    state
        .auth_service
        .role_repo
        .replace_user_roles(user.id, &roles)
        .await?;

    // 只记用户名与角色，**绝不记口令**——哪怕是管理员代设的那一份
    audit.push(format!(
        "新建用户 \"{}\"（{}），角色：{}",
        user.username,
        user.id,
        audit::roles_list(&roles)
    ));
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UserManageRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    // 同建号：先归一再校验，落库与查重都用归一后的值
    let username = validation::normalize_username(&req.username)?;
    let email = validation::normalize_email(&req.email)?;
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

    let before_active = state.auth_service.user_repo.find_by_id(id).await?.is_active;

    let updated = state
        .auth_service
        .user_repo
        .update(id, &username, &email, req.is_active.unwrap_or(true))
        .await?;

    let mut facts: Vec<String> = Vec::new();
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
        let diff = audit::diff_summary(&current_roles, &new_roles, "追加角色", "移除角色");
        facts.push(format!("角色变更（{diff}）"));
    }
    if updated.is_active != before_active {
        let to = if updated.is_active {
            "启用"
        } else {
            "停用"
        };
        facts.push(format!("状态改为{to}"));
    }
    if !facts.is_empty() {
        audit.push(format!(
            "更新用户 \"{}\"（{id}）：{}",
            updated.username,
            facts.join("；")
        ));
    } else {
        audit.push(format!("更新用户 \"{}\"（{id}）", updated.username));
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
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

    // 用户名与角色一起记：行删掉后 `user_roles` 也被级联清空，
    // 只留 UUID 的话，"删掉的是哪个账号、它原本是什么权限"都答不出来
    audit.push(format!(
        "删除用户 \"{}\"（{id}），原角色：{}",
        user.username,
        audit::roles_list(&roles)
    ));
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
    audit: AuditDetail,
    ApiJson(req): ApiJson<BatchDeleteRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    if req.ids.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个用户".to_string()));
    }
    if req.ids.contains(&auth_user.user_id) {
        return Err(AppError::BadRequest("不能删除当前登录账号".to_string()));
    }

    // 先整体校验：逐个删除时无法发现"这一批会删掉全部管理员"
    let mut admins_in_batch = 0i64;
    let mut targets: Vec<(uuid::Uuid, String)> = Vec::with_capacity(req.ids.len());
    for id in &req.ids {
        let user = state.auth_service.user_repo.find_by_id(*id).await?;
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
        targets.push((*id, user.username));
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
    // 逐个记名字而不是只记数量与 ID：批量操作的事后追溯最怕
    // "删了 3 个人"却不知道是哪 3 个
    let names = targets
        .iter()
        .map(|(id, name)| format!("\"{name}\"（{id}）"))
        .collect::<Vec<_>>()
        .join("、");
    audit.push(format!("批量删除 {} 个用户：{names}", targets.len()));
    Ok(Json(ApiResponse::success("批量删除成功")))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BatchDeleteRequest {
    pub ids: Vec<Uuid>,
}

/// POST /api/admin/users/{id}/unlock — 解锁被临时锁定的账号
///
/// 补上 `clear_login_failures` 一直缺的那个调用入口：v0.19.0 之前它
/// **唯一**的调用点在登录成功分支，于是用户被锁只能干等
/// `LOGIN_FAILURE_WINDOW`（默认 300 秒）自然过期，管理员无手动手段。
///
/// 用 POST 而非 PUT：这是一个**动作**，不是把某个字段改成某个值。
/// 重复调用两次与调用一次结果相同，但把它归到 REST 的字段更新里
/// 会诱导前端做"乐观回填"——显示已解锁，而计数其实还在。
#[utoipa::path(
    post,
    path = "/api/admin/users/{id}/unlock",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    responses(
        (status = 200, description = "已解除锁定（cleared_failures 为 0 表示本来就没有锁定）", body = ApiResponse<crate::service::auth::UnlockedAccount>),
        (status = 404, description = "用户不存在"),
    )
)]
pub async fn unlock_user(
    State(state): State<AppState>,
    _perm: PermUserUnlock,
    ApiPath(id): ApiPath<Uuid>,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<crate::service::auth::UnlockedAccount>>, AppError> {
    let user = state.auth_service.user_repo.find_by_id(id).await?;

    let result = state
        .auth_service
        .unlock_user(&state.redis_client, &user)
        .await?;

    // 记录**清掉了多少次**，而不只是"调用了解锁"。
    // 解锁是一次"我确认这个人是本人"的判断，低频但高价值，
    // 事后要能回答"当时到底解了几个桶"。
    audit.push(format!(
        "解锁账号 \"{}\"：清除登录失败计数 {} 次（涉及 {} 个计数桶：用户名 / 邮箱）",
        result.username, result.cleared_failures, result.scopes_cleared
    ));

    Ok(Json(ApiResponse::success(result)))
}

/// GET /api/admin/users/{id}/sessions — 列出该用户的在线会话
///
/// 补上 v0.19.0 之前完全没有的能力：登录成功时**不写任何会话记录**，
/// jti 只在登出时进黑名单，所以"这个人现在在哪些设备上"根本无从回答。
/// 而这正是判断账号是否被盗用的第一手依据。
#[utoipa::path(
    get,
    path = "/api/admin/users/{id}/sessions",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "用户 ID")),
    responses(
        (status = 200, description = "在线会话列表（按登录时间倒序）；用户不存在或当前无在线会话时返回空数组", body = ApiResponse<Vec<crate::service::auth::SessionView>>),
    )
)]
pub async fn list_user_sessions(
    State(state): State<AppState>,
    _perm: PermSessionManage,
    ApiPath(id): ApiPath<Uuid>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<Vec<crate::service::auth::SessionView>>>, AppError> {
    // **刻意不查用户是否存在**，与 `GET /api/admin/users/{id}/roles` 保持一致：
    // 那里对不存在的用户返回 200 + 空数组，这里若回 404，同一个 `{id}` 在
    // 两个"查这个人的附属信息"的端点上就给出两种相反的答案。
    //
    // 代价要认：一个写错或已删除的 id 与"这个人确实没在线"确实长得一样。
    // 换来的是这个端点遵守仓库对读端点的统一约定（不存在即空列表），
    // `every_documented_endpoint_is_reachable_without_a_server_error`
    // 会拿一个**不存在**的 UUID 探针，读端点回 4xx 一律算缺陷。
    //
    // 真的需要区分时，调用方手上已经有用户列表（`GET /api/admin/users` 可按
    // keyword 精确查到），不必靠这个端点去反推 id 是否有效。
    let sessions = state
        .auth_service
        .list_sessions(&state.redis_client, id, &auth_user.token_jti)
        .await?;

    Ok(Json(ApiResponse::success(sessions)))
}

/// POST /api/admin/users/{id}/sessions/{jti}/revoke — 吊销单个会话
#[utoipa::path(
    post,
    path = "/api/admin/users/{id}/sessions/{jti}/revoke",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "用户 ID"),
        ("jti" = String, Path, description = "令牌唯一标识（UUID），取自会话列表"),
    ),
    responses(
        (status = 200, description = "该会话已失效", body = ApiResponse<crate::service::auth::RevokedSession>),
        (status = 404, description = "用户或会话不存在"),
        (status = 400, description = "jti 不是合法的 UUID"),
    )
)]
pub async fn revoke_user_session(
    State(state): State<AppState>,
    perm: PermSessionManage,
    ApiPath((id, jti)): ApiPath<(Uuid, String)>,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<crate::service::auth::RevokedSession>>, AppError> {
    // jti 会直接拼进 Redis 键（`sess:{user_id}:{jti}`），所以必须先校验形状。
    // 不校验的话，路径里的 `*` 或空格能进键名；而列举用的是
    // `SCAN sess:{user_id}:*` 这个 glob 模式——一次键名污染就能让
    // 列表凭空多出别人的会话，或者一个都列不出来。
    //
    // 顺手也防住了路径穿越式的键构造。
    let jti = uuid::Uuid::parse_str(&jti)
        .map_err(|_| AppError::BadRequest("会话标识 jti 必须是合法的 UUID".into()))?;
    let jti = jti.to_string();

    let user = state.auth_service.user_repo.find_by_id(id).await?;

    let result = state
        .auth_service
        .revoke_session(&state.redis_client, id, &jti)
        .await?;

    audit.push(format!(
        "吊销账号 \"{}\" 的单个会话（{}，登录 IP {}），剩余会话 {} 个",
        user.username,
        jti.chars().take(8).collect::<String>() + "…",
        "见在线会话列表",
        result.remaining_sessions
    ));

    let _ = perm;
    Ok(Json(ApiResponse::success(result)))
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<ToggleStatusRequest>,
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
    // 停用是本系统里最容易被用来"掐断某人服务"的开关，
    // 前后状态都记，事后才分得清"刚被停用"和"本来就是停用的"
    let was = if user.is_active { "启用" } else { "停用" };
    let now = if updated.is_active {
        "启用"
    } else {
        "停用"
    };
    audit.push(format!(
        "用户 \"{}\"（{id}）状态由{was}改为{now}，角色：{}",
        user.username,
        audit::roles_list(&roles)
    ));
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<ResetPasswordRequest>,
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

    validation::validate_password_with(
        &req.password,
        &state.setting_service.password_policy().await,
    )?;

    let hashed =
        hash_password(&req.password).map_err(|e| AppError::InternalServerError(e.to_string()))?;
    state
        .auth_service
        .user_repo
        .update_password_hash(id, &hashed)
        .await?;

    // 重置意味着用户**本人没参与**这次口令选择，必须让其改掉
    state
        .auth_service
        .user_repo
        .set_must_change_password(id, true)
        .await?;

    // 口令已变化：吊销该用户全部存量会话
    state
        .auth_service
        .revoke_all_sessions(&state.redis_client, id)
        .await?;

    // 只记"重置了谁的口令"，**新口令一个字都不记**。
    // 这是全库风险最高的写操作（拿到新口令即等于登录成该账号），
    // 也正因如此审计里绝不能出现口令本身
    audit.push(format!(
        "重置用户 \"{}\"（{id}）的口令，已强制其下次登录改密并吊销全部会话",
        user.username
    ));
    tracing::info!("管理员重置用户密码并吊销会话: {}", user.username);
    Ok(Json(ApiResponse::success("密码重置成功")))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ResetPasswordRequest {
    pub password: String,
}

// ──────────────────────────────────────────────
// v0.20.0 B3：CSV 批量导入用户
// ──────────────────────────────────────────────

/// 导入请求体
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportUsersRequest {
    /// CSV 文本内容。**刻意用文本而不是 multipart 上传**：
    /// 这批数据通常来自另一个系统的导出接口，直接把响应体转发过来
    /// 比"下载到本地再选文件"少一步，也少一个失败点。
    /// 表头必需列：`username,email,password,roles`；可选列：`display_name`。
    /// `roles` 单元格内用 `|` 分隔多个角色（如 `user|admin`）。
    pub csv: String,
    /// 只校验不落库
    ///
    /// 导入的真实风险不是"建错号"，而是"建了一百个号发现全部要返工"。
    /// 试运行让管理员在写入前看到逐行的成败与原因。
    #[serde(default)]
    pub dry_run: bool,
}

/// 导入中某一行的失败原因
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ImportRowFailure {
    /// CSV 行号（含表头，从 1 开始），让人能直接定位到那一行
    pub line: usize,
    /// 出问题的用户名；解析失败导致连用户名都取不到时为空串
    pub username: String,
    pub reason: String,
}

/// 导入结果
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ImportUsersResponse {
    /// 解析出的总行数
    pub total: usize,
    pub created: usize,
    pub failed: usize,
    /// 逐行失败原因；成功时为空数组
    ///
    /// **必须回逐行原因而不是只回一个总数**：
    /// 管理员的下一动作是"改第 47 行再导一次"，
    /// 只告诉他"3 行失败"等于让他自己数行号。
    pub failures: Vec<ImportRowFailure>,
    /// 实际建成的用户名
    pub created_usernames: Vec<String>,
    /// 是否为试运行（未落库）
    pub dry_run: bool,
}

/// POST /api/admin/users/import — 从 CSV 批量导入用户
///
/// 补上 v0.20.0 之前 `GET /api/admin/export/users` 的**反向**能力：
/// 那时只有导出没有导入，管理员要给一批同事开号只能一个一个填表单。
///
/// ── 逐行成败而不是整批成败 ──
///
/// 一份 200 行的表里错了一行就整批回滚，管理员既不知道错在哪，
/// 也没法只补那一个用户。因此失败行被跳过并逐条回报，
/// 成功行照常入库。**这也意味着响应 200 不代表全部成功**——
/// 看 `failed` 与 `failures`。
#[utoipa::path(
    post,
    path = "/api/admin/users/import",
    tag = "用户管理",
    security(("bearer_auth" = [])),
    request_body = ImportUsersRequest,
    responses(
        (status = 200, description = "已逐行处理；看 failed 与 failures 判断成败", body = ApiResponse<ImportUsersResponse>),
        (status = 400, description = "CSV 本身无法解析（缺列、只有表头、超过单批上限）"),
        (status = 403, description = "试图授予自己没有的权限码"),
    )
)]
pub async fn import_users(
    State(state): State<AppState>,
    perm: PermUserCreate,
    audit: AuditDetail,
    ApiJson(req): ApiJson<ImportUsersRequest>,
) -> Result<Json<ApiResponse<ImportUsersResponse>>, AppError> {
    use crate::utils::password::hash_password;
    use crate::utils::user_import::parse_user_csv;

    let rows = parse_user_csv(&req.csv)?;

    // 授权下界（v0.5.0 PR-3）：整批先校验。放在循环之外是因为
    // "能建号"不等于"能建管理员"——若逐行校验，第 3 行才撞上越权时，
    // 前两行已经落库，于是管理员既拿到了部分写入，又没拿到明确的拒绝理由。
    let mut all_roles: Vec<String> = Vec::new();
    for row in &rows {
        for name in &row.roles {
            let name = normalize_role_name(name)?;
            if !all_roles.contains(&name) {
                all_roles.push(name);
            }
        }
    }
    ensure_can_grant_roles(
        &state,
        perm.guard(),
        &all_roles,
        &format!("批量导入用户并赋予角色 {}", all_roles.join("、")),
    )
    .await?;

    // 策略在循环外读一次：它在内部会打一次 Redis，
    // 逐行读会让一个 500 行的 CSV 多出 500 次往返。
    // 导入是一次快照语义——同一批里的所有行用同一份策略判定，
    // 这也是管理员能预期的（不会出现"前 10 行按旧策略、后 10 行按新策略"）。
    let password_policy = state.setting_service.password_policy().await;

    let mut created = 0usize;
    let mut created_usernames: Vec<String> = Vec::new();
    let mut failures: Vec<ImportRowFailure> = Vec::new();

    for row in rows {
        let line = row.line;
        let username_raw = row.username.clone();
        let mut fail = |reason: String| {
            failures.push(ImportRowFailure {
                line,
                username: username_raw.clone(),
                reason,
            })
        };

        // 归一与校验的规则**只从 validation 取**，与单个建号完全一致：
        // 导入若自己写一套，第二年两套规则必然走偏
        let username = match validation::normalize_username(&row.username) {
            Ok(v) => v,
            Err(e) => {
                fail(e.to_string());
                continue;
            }
        };
        let email = match validation::normalize_email(&row.email) {
            Ok(v) => v,
            Err(e) => {
                fail(e.to_string());
                continue;
            }
        };
        if let Err(e) = validation::validate_password_with(&row.password, &password_policy) {
            fail(e.to_string());
            continue;
        }
        // 展示名归一后可能变成 None（整格空白），那是"没设过"，不是错误
        let display_name =
            match validation::normalize_display_name(row.display_name.as_deref().unwrap_or("")) {
                Ok(v) => v,
                Err(e) => {
                    fail(e.to_string());
                    continue;
                }
            };
        if row.roles.is_empty() {
            fail("未指定角色：没有角色的用户登录后没有任何权限".to_string());
            continue;
        }

        let roles = match resolve_roles(&state, &row.roles).await {
            Ok(v) => v,
            Err(e) => {
                fail(e.to_string());
                continue;
            }
        };

        // 查重：与单个建号同样在写之前查。若靠数据库唯一约束兜底，
        // 报错会是"duplicate key violates unique constraint \"users_username_key\""，
        // 管理员无从知道该改用户名还是邮箱。
        if state
            .auth_service
            .user_repo
            .find_by_username(&username)
            .await?
            .is_some()
        {
            fail("用户名已被占用".to_string());
            continue;
        }
        if state
            .auth_service
            .user_repo
            .find_by_email(&email)
            .await?
            .is_some()
        {
            fail("邮箱已被占用".to_string());
            continue;
        }

        if req.dry_run {
            created += 1;
            created_usernames.push(username);
            continue;
        }

        let password_hash = match hash_password(&row.password) {
            Ok(h) => h,
            Err(e) => {
                fail(format!("口令加密失败: {e}"));
                continue;
            }
        };

        let user = state
            .auth_service
            .user_repo
            .create(
                Uuid::new_v4(),
                &username,
                &email,
                &password_hash,
                // 与单个建号一致：口令由管理员代设，用户本人从未参与选择，
                // 因此强制其首次登录后改掉（v0.11.0）
                true,
            )
            .await;
        let user = match user {
            Ok(u) => u,
            Err(e) => {
                fail(e.to_string());
                continue;
            }
        };

        // 展示名只能在用户行落库后写：`user_repo::create` 不接受该参数，
        // 而给它加一个可选参数会让单个建号路径也背上"传不传都得处理"的三态。
        // 导入是可接受的窄场景，为它改动通用签名不划算。
        if let Some(name) = display_name {
            if let Err(e) = state
                .auth_service
                .update_profile(user.id, Some(Some(&name)), None)
                .await
            {
                fail(format!(
                    "用户已创建（{user_id}）但展示名写入失败: {e}",
                    user_id = user.id
                ));
                continue;
            }
        }

        if let Err(e) = state
            .auth_service
            .role_repo
            .replace_user_roles(user.id, &roles)
            .await
        {
            // 用户行已经落库却没角色，等于造出一个"登录了但什么都做不了"的账号，
            // 而管理员在结果里只看到一行失败。必须说出来。
            fail(format!(
                "用户已创建（{user_id}）但角色写入失败，需人工处理: {e}",
                user_id = user.id
            ));
            continue;
        }

        created += 1;
        created_usernames.push(username);
    }

    // 审计记**建成了谁**，不记口令。批量场景下这串用户名是事后清理的依据，
    // 而口令一旦进了这张长期表就是一次全库泄露
    let summary = if created_usernames.is_empty() {
        "无".to_string()
    } else {
        created_usernames.join("、")
    };
    audit.push(format!(
        "批量导入用户：共 {} 行，成功 {}，失败 {}；{}{}",
        created + failures.len(),
        created,
        failures.len(),
        if req.dry_run {
            "（试运行，未落库）"
        } else {
            ""
        },
        summary
    ));
    tracing::info!(
        "批量导入用户: {} 行，成功 {}，失败 {}{}",
        created + failures.len(),
        created,
        failures.len(),
        if req.dry_run { "（试运行）" } else { "" }
    );

    Ok(Json(ApiResponse::success(ImportUsersResponse {
        total: created + failures.len(),
        created,
        failed: failures.len(),
        failures,
        created_usernames,
        dry_run: req.dry_run,
    })))
}
