//! 部门管理控制器
//!
//! 提供管理员操作部门的接口：树形列表、扁平列表、新建、修改、移动、删除。
//!
//! ── 为什么有树形和扁平两个列表端点 ──────────────────────────
//! 树形用于页面展示（`n-tree` 组件直接消费），
//! 扁平用于下拉选择（新建/移动部门时选父部门）。
//! 两者的 `level` 和 `path` 字段让前端能显示缩进和完整路径。
//!
//! ── 路由顺序 ────────────────────────────────────────────────
//! `flat` / `move` / `users` 这三个静态段**必须**排在 `{id}` 之前，
//! 否则它们会被当成一个 id 走进 `{id}` 端点，然后以"部门不存在"404——
//! 一个看起来像数据错误、实际是路由根本没匹配上的响应。

use axum::{extract::State, Json};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::permission::{PermDeptCreate, PermDeptDelete, PermDeptList, PermDeptUpdate};
use crate::model::department::{
    CreateDepartmentRequest, DepartmentFlat, DepartmentNode, DepartmentUser, MoveDepartmentRequest,
    UpdateDepartmentRequest,
};
use crate::model::{ApiResponse, ChangeType, TargetType};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};

/// GET /api/admin/departments — 部门树
#[utoipa::path(
    get,
    path = "/api/admin/departments",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "部门树（根部门按 sort_order 升序，子部门递归嵌套）", body = ApiResponse<Vec<DepartmentNode>>),
    )
)]
pub async fn list_departments(
    State(state): State<AppState>,
    _perm: PermDeptList,
) -> Result<Json<ApiResponse<Vec<DepartmentNode>>>, AppError> {
    let tree = state.department_service.tree().await?;
    Ok(Json(ApiResponse::success(tree)))
}

/// GET /api/admin/departments/flat — 扁平列表（用于下拉选择）
#[utoipa::path(
    get,
    path = "/api/admin/departments/flat",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "扁平部门列表（含 level 与 path，用于下拉选择）", body = ApiResponse<Vec<DepartmentFlat>>),
    )
)]
pub async fn list_departments_flat(
    State(state): State<AppState>,
    _perm: PermDeptList,
) -> Result<Json<ApiResponse<Vec<DepartmentFlat>>>, AppError> {
    let list = state.department_service.flat_list().await?;
    Ok(Json(ApiResponse::success(list)))
}

/// POST /api/admin/departments — 新建部门
#[utoipa::path(
    post,
    path = "/api/admin/departments",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    request_body = CreateDepartmentRequest,
    responses(
        (status = 200, description = "部门已创建", body = ApiResponse<crate::model::department::Department>),
        (status = 400, description = "名称长度不合法、父部门不存在、或同一父部门下名称重复"),
    )
)]
pub async fn create_department(
    State(state): State<AppState>,
    _perm: PermDeptCreate,
    audit: AuditDetail,
    ApiJson(req): ApiJson<CreateDepartmentRequest>,
) -> Result<Json<ApiResponse<crate::model::department::Department>>, AppError> {
    let dept = state.department_service.create(&req).await?;
    audit.push_targeted(
        format!(
            "新建部门 \"{}\"（父部门 {}）",
            dept.name,
            dept.parent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "无（根部门）".to_string())
        ),
        TargetType::Department,
        dept.id,
        ChangeType::Create,
        Some(dept.name.clone()),
    );
    Ok(Json(ApiResponse::success(dept)))
}

/// PUT /api/admin/departments/{id} — 修改部门（名称 / 描述 / 排序）
#[utoipa::path(
    put,
    path = "/api/admin/departments/{id}",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "部门 ID")),
    request_body = UpdateDepartmentRequest,
    responses(
        (status = 200, description = "部门已更新", body = ApiResponse<crate::model::department::Department>),
        (status = 404, description = "部门不存在"),
        (status = 400, description = "名称长度不合法或同一父部门下名称重复"),
    )
)]
pub async fn update_department(
    State(state): State<AppState>,
    _perm: PermDeptUpdate,
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateDepartmentRequest>,
) -> Result<Json<ApiResponse<crate::model::department::Department>>, AppError> {
    let dept = state.department_service.update(id, &req).await?;
    audit.push_targeted(
        format!("修改部门 \"{}\"", dept.name),
        TargetType::Department,
        dept.id,
        ChangeType::Update,
        Some(dept.name.clone()),
    );
    Ok(Json(ApiResponse::success(dept)))
}

/// POST /api/admin/departments/{id}/move — 移动部门
#[utoipa::path(
    post,
    path = "/api/admin/departments/{id}/move",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "部门 ID")),
    request_body = MoveDepartmentRequest,
    responses(
        (status = 200, description = "部门已移动", body = ApiResponse<crate::model::department::Department>),
        (status = 404, description = "部门不存在"),
        (status = 400, description = "目标父部门不存在，或不能移动到自己/子孙下面"),
    )
)]
pub async fn move_department(
    State(state): State<AppState>,
    _perm: PermDeptUpdate,
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<MoveDepartmentRequest>,
) -> Result<Json<ApiResponse<crate::model::department::Department>>, AppError> {
    let dept = state.department_service.move_to(id, &req).await?;
    audit.push_targeted(
        format!(
            "移动部门 \"{}\" 到 {}",
            dept.name,
            dept.parent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "根".to_string())
        ),
        TargetType::Department,
        dept.id,
        ChangeType::Update,
        Some(dept.name.clone()),
    );
    Ok(Json(ApiResponse::success(dept)))
}

/// DELETE /api/admin/departments/{id} — 删除部门
#[utoipa::path(
    delete,
    path = "/api/admin/departments/{id}",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "部门 ID")),
    responses(
        (status = 200, description = "部门已删除（该部门下用户的 dept_id 已变成 NULL）", body = ApiResponse<String>),
        (status = 404, description = "部门不存在"),
        (status = 400, description = "该部门下还有子部门，请先移动或删除它们"),
    )
)]
pub async fn delete_department(
    State(state): State<AppState>,
    _perm: PermDeptDelete,
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    // 名字必须在删之前取。删完之后 `departments` 行没了，
    // 审计里就只剩一个 UUID——"删掉的是哪个部门"再也答不出来。
    // 这与 `utils/audit.rs` 文件头那条规矩是同一件事，
    // 此前的实现漏了它，这里补上。
    let name = state
        .department_repo
        .find_by_id(id)
        .await
        .ok()
        .and_then(|d| d.map(|d| d.name));
    state.department_service.delete(id).await?;
    audit.push_targeted(
        format!(
            "删除部门 {id}{}",
            name.as_deref()
                .map(|n| format!("（\"{n}\")"))
                .unwrap_or_default()
        ),
        TargetType::Department,
        id,
        ChangeType::Delete,
        name,
    );
    Ok(Json(ApiResponse::success("部门已删除".to_string())))
}

/// GET /api/admin/departments/{id}/users — 该部门下的用户
#[utoipa::path(
    get,
    path = "/api/admin/departments/{id}/users",
    tag = "部门管理",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "部门 ID")),
    responses(
        (status = 200, description = "该部门下的用户列表；部门不存在时返回空数组", body = ApiResponse<Vec<DepartmentUser>>),
    )
)]
pub async fn list_department_users(
    State(state): State<AppState>,
    _perm: PermDeptList,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<Vec<DepartmentUser>>>, AppError> {
    let users = state.department_service.list_users(id).await?;
    Ok(Json(ApiResponse::success(users)))
}
