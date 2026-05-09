//! 菜单管理控制器

use axum::{extract::{Path, Query, State}, Json};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{ApiResponse, Menu, MenuNode, CreateMenuRequest, UpdateMenuRequest, AssignMenuRequest};
use crate::router::AppState;
use crate::utils::validation;

/// GET /api/admin/menus — 获取菜单树
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

#[derive(Debug, serde::Deserialize)]
pub struct MenuQuery {
    pub role_id: Option<String>,
}

/// POST /api/admin/menus — 新增菜单
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
pub async fn update_menu(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMenuRequest>,
) -> Result<Json<ApiResponse<MenuNode>>, AppError> {
    let saved = state.menu_repo.update(id, &req).await?;
    Ok(Json(ApiResponse::success(MenuNode::from(saved))))
}

/// DELETE /api/admin/menus/:id — 删除菜单
pub async fn delete_menu(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.menu_repo.delete(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// PUT /api/admin/roles/:id/menus — 分配角色菜单权限
pub async fn assign_role_menus(
    State(state): State<AppState>,
    Path(role_id): Path<Uuid>,
    Json(req): Json<AssignMenuRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.menu_repo.assign_role_menus(role_id, &req.menu_ids).await?;
    Ok(Json(ApiResponse::success("权限分配成功")))
}
