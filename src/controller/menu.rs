//! 菜单管理控制器

use axum::{
    extract::rejection::QueryRejection,
    extract::{Query, State},
    Json,
};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::permission::{
    PermMenuCreate, PermMenuDelete, PermMenuGrant, PermMenuList, PermMenuUpdate,
};
use crate::model::{
    ApiResponse, AssignMenuRequest, ChangeType, CreateMenuRequest, Menu, MenuNode, TargetType,
    UnreachableMenu, UpdateMenuRequest,
};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};
use crate::utils::audit;
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
    params: Result<Query<MenuQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<MenuNode>>>, AppError> {
    // 显式接住拒绝，错误才走统一响应格式（见 `From<QueryRejection>`）
    let Query(params) = params?;
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
#[serde(deny_unknown_fields)]
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

/// GET /api/admin/menus/diagnostics — 走不到根、因而不在任何菜单树里的菜单
///
/// 成环（或悬空引用）的菜单会被 `build_tree` **静默剪掉**：
/// 侧栏和管理页看不到它，管理员也就无法点开改回来，只能直连数据库救。
/// 本接口把这些节点显出来，让"看不见"变成"看得见并能修"。
///
/// 修复动作复用既有公开 API：把它挂到根下（`parent_id: null`）即可，
/// 所以这里只读、不提供第二条写路径。
#[utoipa::path(
    get,
    path = "/api/admin/menus/diagnostics",
    tag = "菜单管理",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "结构诊断结果", body = ApiResponse<Vec<UnreachableMenu>>)
    )
)]
pub async fn menu_diagnostics(
    State(state): State<AppState>,
    _perm: PermMenuList,
) -> Result<Json<ApiResponse<Vec<UnreachableMenu>>>, AppError> {
    let broken = state.menu_repo.find_unreachable().await?;
    if !broken.is_empty() {
        tracing::warn!(
            count = broken.len(),
            "菜单树存在不可达节点（成环或悬空引用），它们不会出现在任何菜单树里"
        );
    }
    Ok(Json(ApiResponse::success(broken)))
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
    audit: AuditDetail,
    ApiJson(req): ApiJson<CreateMenuRequest>,
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
    audit.push_targeted(
        match saved.permission.as_deref().filter(|p| !p.is_empty()) {
            Some(code) => format!(
                "新建菜单 \"{}\"（{}），声明权限码 \"{code}\"",
                saved.name, saved.id
            ),
            None => format!("新建菜单 \"{}\"（{}），不携带权限码", saved.name, saved.id),
        },
        TargetType::Menu,
        saved.id,
        ChangeType::Create,
        Some(saved.name.clone()),
    );
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateMenuRequest>,
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

    // 改权限码之前先记住旧值：`menus.permission` 改写后旧码就查不到了，
    // 而"这个码从谁手里转移到了谁手里"正是授权追溯要回答的问题
    let before = state.menu_repo.find_by_id(id).await?;
    let before_code = before
        .permission
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(str::to_string);

    let saved = state.menu_repo.update(id, &req, auth_user.user_id).await?;
    let after_code = saved
        .permission
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    audit.push_targeted(
        audit::permission_change(
            &audit::label("菜单", &saved.name),
            saved.id,
            before_code.as_deref(),
            after_code.as_deref(),
        ),
        TargetType::Menu,
        saved.id,
        ChangeType::Update,
        Some(saved.name.clone()),
    );

    // 父级变更是**结构性**变更，此前审计完全不留痕：
    // 把一个目录挪到别处，它整棵子树的导航归属就变了，而审计里只有一行
    // "无权限码变更"。这里补上，且说清是"挪走"还是"摘成根"。
    if let Some(new_parent) = saved.parent_id {
        if before.parent_id != Some(new_parent) {
            let parent_name = state
                .menu_repo
                .find_by_id(new_parent)
                .await
                .map(|m| m.name)
                .unwrap_or_else(|_| new_parent.to_string());
            // 旧上级也用名字：审计是用来事后读的，裸 UUID 逼人去翻库
            let old_parent_desc = match before.parent_id {
                Some(old) => state
                    .menu_repo
                    .find_by_id(old)
                    .await
                    .map(|m| format!("\"{}\"（{}）", m.name, old))
                    .unwrap_or_else(|_| old.to_string()),
                None => "根".to_string(),
            };
            // `Menu/Update` 这个 target 会被去重合并成一条——
            // 上面那条 `permission_change` 已经声明过它了。
            // 两条摘要文本都留着（一条说改码、一条说挪位置），但结构化侧
            // 只需要"这个菜单被更新过"这一个事实。
            audit.push_targeted(
                format!(
                    "移动菜单 \"{}\"（{}）：上级从 {} 改为 \"{}\"（{}）",
                    saved.name, saved.id, old_parent_desc, parent_name, new_parent
                ),
                TargetType::Menu,
                saved.id,
                ChangeType::Update,
                Some(saved.name.clone()),
            );
        }
    } else if let Some(old_parent) = before.parent_id {
        // `parent_id: null` 现在真的生效了（此前返回 200 却什么也没做）
        audit.push_targeted(
            format!(
                "移动菜单 \"{}\"（{}）：摘成根菜单，不再挂在上级 {} 下",
                saved.name, saved.id, old_parent
            ),
            TargetType::Menu,
            saved.id,
            ChangeType::Update,
            Some(saved.name.clone()),
        );
    }
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
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
    audit.push_targeted(
        format!(
            "恢复菜单 \"{}\"（{}）的权限码 \"{restorable}\"",
            restored.name, restored.id
        ),
        TargetType::Menu,
        restored.id,
        ChangeType::Grant,
        Some(restored.name.clone()),
    );
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
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

    // 名字要在 delete 之前取：删除是级联的（子树 + 全部 role_menus），
    // 之后 `menus` 行已不存在，"删的是哪个菜单"就再也答不出来了
    let name = state
        .menu_repo
        .find_by_id(id)
        .await
        .map(|m| m.name)
        .unwrap_or_else(|_| format!("<{id}>"));
    state.menu_repo.delete(id).await?;
    let revoked = audit::codes("随之从角色收回的权限码", &granted_codes);
    audit.push_targeted(
        match revoked.is_empty() {
            true => format!("删除菜单 \"{name}\"（{id}）"),
            false => format!("删除菜单 \"{name}\"（{id}）；{revoked}"),
        },
        TargetType::Menu,
        id,
        ChangeType::Delete,
        // 菜单行已随级联删除消失，这一列是"删的是哪个按钮"的最后存档
        Some(name.clone()),
    );
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
    audit: AuditDetail,
    ApiPath(role_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<AssignMenuRequest>,
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

    // 变更前的权限码快照。本接口是**全量替换**语义，
    // 所以"这次变了什么"只能靠前后两个集合求差得到——
    // 只记提交上来的 `menu_ids`，事后仍答不出"撤了哪些"。
    // 这里查的是**权限码**而不是菜单 ID：目录/页面菜单不带码，
    // 把它们算进变更会制造一堆没有权限含义的噪声。
    let before_codes = state.menu_repo.permission_codes_of_role(role_id).await?;

    state
        .menu_repo
        .assign_role_menus(role_id, &req.menu_ids)
        .await?;

    let after_codes = state.menu_repo.permission_codes_of_role(role_id).await?;
    let role_name = target_role_name
        .clone()
        .unwrap_or_else(|| format!("<{role_id}>"));
    let diff = audit::diff_summary(&before_codes, &after_codes, "授予权限码", "撤销权限码");
    // 结构化对象：角色本身 + 这次真正动过的那些菜单（v0.26.0）
    //
    // 菜单 target 取自**求差后的权限码**反查，而不是提交的 `req.menu_ids`：
    // 全量替换语义下，提交上来的集合里绝大多数菜单本来就有权限，
    // 拿它当 target 会让"这个按钮被谁动过"答出一堆没动过的按钮。
    let (added_codes, removed_codes) = audit::diff_sets(&before_codes, &after_codes);
    let touched = state
        .menu_repo
        .find_menu_ids_by_permission_codes(&[added_codes.clone(), removed_codes.clone()].concat())
        .await?;

    let line = match diff.is_empty() {
        // 重复提交同一份集合：什么都没变，如实记成"无变化"而不是伪造一次授权
        true => format!("角色 \"{role_name}\"（{role_id}）的权限码无变化"),
        false => format!("角色 \"{role_name}\"（{role_id}）权限码变更：{diff}"),
    };
    // 无变化时不声明任何 target：这次什么都没动，
    // 往 target 表里写一行会让"这个按钮被改过"出现假阳性。
    if diff.is_empty() {
        audit.push(line);
    } else {
        audit.push_targeted(
            line,
            TargetType::Role,
            role_id,
            // 授予与撤销同时发生时记 `grant`：这一条回答的是
            // "这个角色的权限被谁动过"，而动它的**动作**就是授权变更。
            // 逐菜单的授予/撤销方向由下面每个菜单 target 自己表达。
            ChangeType::Grant,
            Some(role_name.clone()),
        );
        for (menu_id, code) in touched {
            let change = if added_codes.contains(&code) {
                ChangeType::Grant
            } else {
                ChangeType::Revoke
            };
            audit.add_target(TargetType::Menu, menu_id, change, Some(code));
        }
    }
    Ok(Json(ApiResponse::success("权限分配成功")))
}
