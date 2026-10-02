//! 菜单管理控制器

use axum::{
    extract::{Path, Query, State},
    Json,
};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::permission::{
    PermMenuCreate, PermMenuDelete, PermMenuGrant, PermMenuList, PermMenuUpdate,
};
use crate::model::{
    ApiResponse, AssignMenuRequest, CreateMenuRequest, Menu, MenuNode, UpdateMenuRequest,
};
use crate::router::AppState;
use crate::utils::validation;

/// GET /api/admin/menus — 获取菜单树
#[utoipa::path(
    get,
    path = "/api/admin/menus",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    params(("role_id" = Option<String>, Query, description = "按角色过滤菜单树")),
    responses((status = 200, description = "菜单树", body = ApiResponse<Vec<MenuNode>>))
)]
pub async fn list_menus(
    State(state): State<AppState>,
    _perm: PermMenuList,
    Query(params): Query<MenuQuery>,
) -> Result<Json<ApiResponse<Vec<MenuNode>>>, AppError> {
    let repo = &state.menu_repo;
    let tree = if let Some(role_id) = params.role_id {
        validation::validate_uuid(&role_id.to_string())?;
        let rid = uuid::Uuid::parse_str(&role_id).unwrap();
        repo.find_tree_by_role(rid).await?
    } else {
        repo.find_tree().await?
    };
    Ok(Json(ApiResponse::success(tree)))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct MenuQuery {
    pub role_id: Option<String>,
}

/// GET /api/auth/menus — 当前登录用户可见的导航菜单树
///
/// 前端据此动态生成路由与侧栏，因此只返回：
/// 该用户所有角色关联的、`is_visible = true` 的非按钮菜单。
#[utoipa::path(
    get,
    path = "/api/auth/menus",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户的导航菜单树", body = ApiResponse<Vec<MenuNode>>))
)]
pub async fn my_menus(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<Vec<MenuNode>>>, AppError> {
    let mut role_ids = Vec::with_capacity(auth_user.roles.len());
    for role_name in &auth_user.roles {
        if let Some(role) = state.auth_service.role_repo.find_by_name(role_name).await? {
            role_ids.push(role.id);
        }
    }

    if role_ids.is_empty() {
        tracing::warn!(
            "用户 {} 未匹配到任何角色，返回空菜单: {:?}",
            auth_user.user_id,
            auth_user.roles
        );
        return Ok(Json(ApiResponse::success(Vec::new())));
    }

    let tree = state.menu_repo.find_tree_for_roles(&role_ids).await?;
    Ok(Json(ApiResponse::success(tree)))
}

/// GET /api/auth/permissions — 当前登录用户的权限码
///
/// 前端 `v-permission` / `PermissionButton` 据此判定按钮级权限，
/// 与后端 `PermissionGuard` 用的是同一份数据源（`menus.permission`）。
#[utoipa::path(
    get,
    path = "/api/auth/permissions",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户的权限码列表", body = ApiResponse<Vec<String>>))
)]
pub async fn my_permissions(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let codes = state
        .menu_repo
        .find_permission_codes(&auth_user.roles)
        .await?;
    Ok(Json(ApiResponse::success(codes)))
}

/// POST /api/admin/menus — 新增菜单
#[utoipa::path(
    post,
    path = "/api/admin/menus",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    request_body = CreateMenuRequest,
    responses((status = 200, description = "创建成功", body = ApiResponse<MenuNode>))
)]
pub async fn create_menu(
    State(state): State<AppState>,
    _perm: PermMenuCreate,
    Json(req): Json<CreateMenuRequest>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
    // 权限码必须唯一：迁移 `007` 的部分唯一索引 `idx_menus_permission_unique`
    // 会挡住重复声明，但索引抛出来的是 500 "服务器内部错误"——
    // 入参错误被当成服务端故障，既污染错误监控，管理员也看不懂发生了什么。
    // 这里先查一次，给出可操作的消息；索引仍作为并发下的最终兜底。
    if let Some(code) = req.permission.as_deref().filter(|p| !p.is_empty()) {
        if state.menu_repo.is_permission_taken(code).await? {
            return Err(AppError::Conflict(format!(
                "权限码「{code}」已被其他菜单使用，请换一个未被占用的码"
            )));
        }
    }

    let menu = Menu {
        id: Uuid::new_v4(),
        parent_id: req.parent_id,
        name: req.name,
        path: req.path,
        component: req.component,
        icon: req.icon,
        sort_order: req.sort_order.unwrap_or(0),
        r#type: req.r#type,
        permission: req.permission,
        is_visible: req.is_visible.unwrap_or(true),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        // 新建的按钮从未被清空过，没有可恢复的码
        prev_permission: None,
        prev_permission_cleared_by: None,
    };
    let saved = state.menu_repo.create(&menu).await?;
    Ok(Json(ApiResponse::success(MenuNode::from(saved))))
}

/// PUT /api/admin/menus/:id — 更新菜单
#[utoipa::path(
    put,
    path = "/api/admin/menus/{id}",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "菜单 ID")),
    request_body = UpdateMenuRequest,
    responses((status = 200, description = "更新成功", body = ApiResponse<MenuNode>))
)]
pub async fn update_menu(
    State(state): State<AppState>,
    perm: PermMenuUpdate,
    auth_user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMenuRequest>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
    // 授权下界（v0.5.0 PR-3）：`menus.permission` 是权限码本身。
    // 把一个**已授权给调用者的**按钮菜单的 permission 改掉，
    // 等于让"角色→菜单→权限码"这条链在自己身上当场生效：
    // 持 `system:menu:update` 的角色可以把已授权按钮改成 `system:user:delete`，
    // 不需要 `menu:grant`，也不需要新建菜单——绕过授予下界的旁路。
    // 因此改写 permission 必须持有目标码。
    if let Some(new_permission) = req.permission.as_deref().filter(|p| !p.is_empty()) {
        let required = vec![new_permission.to_string()];
        perm.guard()
            .ensure_covers(&required, "把菜单的权限码改为该值")?;
    }

    // 清空同样要过守卫，但只在**确实改变了别人权限**时。
    //
    // 此前 `.filter(|p| !p.is_empty())` 让清空整个绕过了检查，于是
    // 持 `system:menu:update` 的角色可以把**别的角色**已持有按钮的码清掉，
    // 绕过 `system:menu:grant` 完成一次跨角色撤权。
    //
    // 而如果这个按钮没授予任何角色，清空不改变任何人的权限，
    // 属于"整理菜单结构"这类无害操作，不该被拦
    // （既有测试 `rewriting_a_granted_menu_permission_to_an_unheld_code_is_denied`
    // 的第一步就依赖这一点）。
    if req.permission.as_deref().is_some_and(|p| p.is_empty()) {
        let current = state.menu_repo.find_by_id(id).await?;
        if let Some(code) = current.permission.as_deref().filter(|p| !p.is_empty()) {
            if state.menu_repo.is_granted_to_any_role(id).await? {
                perm.guard().ensure_covers(
                    &[code.to_string()],
                    &format!("清空已授权菜单的权限码「{code}」"),
                )?;
            }
        }
    }

    let saved = state.menu_repo.update(id, &req, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(MenuNode::from(saved))))
}

/// POST /api/admin/menus/:id/restore-permission — 恢复被清空的权限码
///
/// 权限码被清空后全系统就没有任何角色再持有它，而 `update_menu` 的守卫
/// 要求"改写权限码必须持有目标码"——于是**写回去会被自己的守卫拦死**。
/// 本接口是那条死路唯一的出口。
///
/// ## 为什么只允许"本人恢复本人清掉的"是安全的
///
/// 清空一个**已授权**按钮的码要求调用者持有该码（见 `update_menu`），
/// 因此"能清空"蕴含"清空前持有"。恢复只是把状态还原到清空之前，
/// **净零提权**。若按钮本就未授予任何角色，清空放行、恢复也只是给
/// "无人"一个码，同样净零。
///
/// 反过来，"把菜单授予别的角色"仍要过 `system:menu:grant` 与
/// `assign_role_menus` 的既有判定，所以这里不构成新的越权原语。
#[utoipa::path(
    post,
    path = "/api/admin/menus/{id}/restore-permission",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "菜单 ID")),
    responses(
        (status = 200, description = "恢复成功", body = ApiResponse<MenuNode>),
        (status = 400, description = "没有可恢复的权限码，或不是本人清空的")
    )
)]
pub async fn restore_menu_permission(
    State(state): State<AppState>,
    _perm: PermMenuUpdate,
    auth_user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
    let menu = state.menu_repo.find_by_id(id).await?;
    let Some(restorable) = menu.prev_permission.as_deref() else {
        return Err(AppError::BadRequest(
            "该菜单没有可恢复的权限码：它当前仍持有权限码，或从未被清空过".into(),
        ));
    };
    if menu.prev_permission_cleared_by != Some(auth_user.user_id) {
        return Err(AppError::BadRequest(format!(
            "权限码「{restorable}」不是你清空的，只能由清空者本人恢复；\
             如需接管，请新建按钮并声明该权限码"
        )));
    }
    let restored = state.menu_repo.restore_permission(id).await?;
    tracing::info!("管理员恢复菜单权限码: {} (码: {restorable})", restored.name);
    Ok(Json(ApiResponse::success(MenuNode::from(restored))))
}

/// DELETE /api/admin/menus/:id — 删除菜单
#[utoipa::path(
    delete,
    path = "/api/admin/menus/{id}",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "菜单 ID")),
    responses((status = 200, description = "删除成功", body = ApiResponse<String>))
)]
pub async fn delete_menu(
    State(state): State<AppState>,
    perm: PermMenuDelete,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 授权下界（v0.8.0 第 1 项）：删除是级联的，删掉一个**承载权限码的菜单**
    // 等价于把那个码从所有依赖它的角色身上剥掉，与 `update_menu` 清空该码的
    // 效果完全一样。v0.7.0 已给 update 装了这道守卫，delete 这条路当时没管，
    // 于是同一件事有两个入口、一个拦住一个放行。
    //
    // 守卫覆盖**整棵子树**：`menus.parent_id` 是 `ON DELETE CASCADE`，
    // 删父目录会连带删掉子树里承载码的按钮，只看目标节点会漏掉这条路。
    //
    // 只对"已被授予至少一个角色"的码设限：没人依赖的码删掉不改变任何人的权限
    // （与 v0.7.0 的"清空无害"同理），否则"整理菜单结构"这类无害操作会全线报错。
    let granted_codes = state.menu_repo.granted_codes_in_subtree(id).await?;
    if !granted_codes.is_empty() {
        perm.guard().ensure_covers(
            &granted_codes,
            &format!("删除承载权限码「{}」的菜单", granted_codes.join("、")),
        )?;
    }

    state.menu_repo.delete(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// PUT /api/admin/roles/:id/menus — 分配角色菜单权限
#[utoipa::path(
    put,
    path = "/api/admin/roles/{role_id}/menus",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    params(("role_id" = Uuid, Path, description = "角色 ID")),
    request_body = AssignMenuRequest,
    responses((status = 200, description = "角色菜单已更新", body = ApiResponse<String>))
)]
pub async fn assign_role_menus(
    State(state): State<AppState>,
    perm: PermMenuGrant,
    auth_user: AuthenticatedUser,
    Path(role_id): Path<Uuid>,
    Json(req): Json<AssignMenuRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 授权下界（v0.5.0 PR-3）：`system:menu:grant` 是"把权限码授予角色"的元能力，
    // 持有它就能把全部按钮菜单授予某个角色，因此**自授**必须拦住。
    //
    // 判定只针对"调用者自己的角色"，而不是"全部授权"：
    //
    // - 授给**别的**角色不会让调用者变强。而且这条不能禁——admin 造一个新权限码
    //   再分发给各角色，正是"权限码即数据"的核心工作流；若要求"只能授予自己
    //   已持有的码"，admin 连自己刚造的码都发不出去。
    // - 间接路径仍然闭合：先授给别的角色、之后该角色被授给调用者时，
    //   `ensure_can_grant_roles` 的包含关系判定会按"目标角色的码 ⊆ 你的码"拦住。
    //
    // 只有 `type='button'` 的行携带权限码（见 repo 层的过滤条件）；
    // 目录/页面菜单只影响导航可见性，不授予接口能力，故不受此限。
    let target_role_name = state
        .auth_service
        .role_repo
        .find_name_by_id(role_id)
        .await?;
    if let Some(name) = target_role_name.as_deref() {
        if auth_user.roles.iter().any(|r| r == name) {
            let granted_codes = state
                .menu_repo
                .find_permission_codes_by_menu_ids(&req.menu_ids)
                .await?;
            perm.guard()
                .ensure_covers(&granted_codes, "把该菜单集合授予自己的角色")?;
        }
    }

    state
        .menu_repo
        .assign_role_menus(role_id, &req.menu_ids)
        .await?;
    Ok(Json(ApiResponse::success("权限分配成功")))
}
