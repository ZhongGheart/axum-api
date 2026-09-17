//! 菜单管理控制器

use axum::{
    extract::{Path, Query, State},
    Json,
};
use uuid::Uuid;

use crate::error::AppError;
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
    Json(req): Json<CreateMenuRequest>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
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
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMenuRequest>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
    let saved = state.menu_repo.update(id, &req).await?;
    Ok(Json(ApiResponse::success(MenuNode::from(saved))))
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
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
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
    Path(role_id): Path<Uuid>,
    Json(req): Json<AssignMenuRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state
        .menu_repo
        .assign_role_menus(role_id, &req.menu_ids)
        .await?;
    Ok(Json(ApiResponse::success("权限分配成功")))
}
