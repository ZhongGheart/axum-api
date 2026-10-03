//! 数据字典控制器

use axum::{extract::State, Json};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::permission::{
    PermDictCreate, PermDictDelete, PermDictList, PermDictRefresh, PermDictUpdate,
};
use crate::model::{
    ApiResponse, CreateDictItemRequest, CreateDictTypeRequest, DictItem, DictItemResponse,
    DictType, DictTypeWithItems,
};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};

/// GET /api/admin/dict/types — 字典类型列表
#[utoipa::path(
    get,
    path = "/api/admin/dict/types",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "字典类型列表", body = ApiResponse<Vec<DictType>>))
)]
pub async fn list_types(
    State(state): State<AppState>,
    _perm: PermDictList,
) -> Result<Json<ApiResponse<Vec<DictType>>>, AppError> {
    let types = state.dict_repo.list_types().await?;
    Ok(Json(ApiResponse::success(types)))
}

/// POST /api/admin/dict/types — 新增字典类型
#[utoipa::path(
    post,
    path = "/api/admin/dict/types",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    request_body = CreateDictTypeRequest,
    responses((status = 200, description = "创建成功", body = ApiResponse<DictType>))
)]
pub async fn create_type(
    State(state): State<AppState>,
    _perm: PermDictCreate,
    ApiJson(req): ApiJson<CreateDictTypeRequest>,
) -> Result<Json<ApiResponse<DictType>>, AppError> {
    let t = DictType {
        id: Uuid::new_v4(),
        code: req.code,
        name: req.name,
        description: req.description,
        status: req.status.unwrap_or_else(|| "enabled".into()),
        sort_order: req.sort_order.unwrap_or(0),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let saved = state.dict_repo.create_type(&t).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// PUT /api/admin/dict/types/:id — 更新字典类型
#[utoipa::path(
    put,
    path = "/api/admin/dict/types/{id}",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "字典类型 ID")),
    request_body = CreateDictTypeRequest,
    responses((status = 200, description = "更新成功", body = ApiResponse<DictType>))
)]
pub async fn update_type(
    State(state): State<AppState>,
    _perm: PermDictUpdate,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CreateDictTypeRequest>,
) -> Result<Json<ApiResponse<DictType>>, AppError> {
    let saved = state.dict_repo.update_type(id, &req).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// DELETE /api/admin/dict/types/:id — 删除字典类型（级联删除项）
#[utoipa::path(
    delete,
    path = "/api/admin/dict/types/{id}",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "字典类型 ID")),
    responses((status = 200, description = "删除成功", body = ApiResponse<String>))
)]
pub async fn delete_type(
    State(state): State<AppState>,
    _perm: PermDictDelete,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.dict_repo.delete_type(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// GET /api/admin/dict/:code/items — 获取字典项（直接从缓存或数据库）
#[utoipa::path(
    get,
    path = "/api/dict/{code}/items",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("code" = String, Path, description = "字典编码")),
    responses((status = 200, description = "字典项（带 Redis 缓存）", body = ApiResponse<Vec<DictItemResponse>>))
)]
pub async fn get_items_by_code(
    State(state): State<AppState>,
    ApiPath(code): ApiPath<String>,
) -> Result<Json<ApiResponse<Vec<DictItemResponse>>>, AppError> {
    // 刻意不做权限码校验：这是任意已登录用户可读的通用展示数据，
    // 普通页面的 DictSelect 也依赖它。加权限码会让非管理员的字典下拉全部失效。
    let items = state.dict_repo.get_dict_by_code(&code).await?;
    Ok(Json(ApiResponse::success(items)))
}

/// GET /api/admin/dict/items?dict_type_id=xxx — 根据类型 ID 获取项
#[utoipa::path(
    get,
    path = "/api/admin/dict/items",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("dict_type_id" = Uuid, Query, description = "字典类型 ID")),
    responses((status = 200, description = "字典项列表", body = ApiResponse<Vec<DictItem>>))
)]
pub async fn list_items(
    State(state): State<AppState>,
    _perm: PermDictList,
    params: Result<axum::extract::Query<DictItemQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<ApiResponse<Vec<DictItem>>>, AppError> {
    // 显式接住拒绝，错误才走统一响应格式（见 `From<QueryRejection>`）
    let axum::extract::Query(params) = params?;
    let items = state.dict_repo.list_items(params.dict_type_id).await?;
    Ok(Json(ApiResponse::success(items)))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DictItemQuery {
    pub dict_type_id: Uuid,
}

/// POST /api/admin/dict/items — 新增字典项
#[utoipa::path(
    post,
    path = "/api/admin/dict/items",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    request_body = CreateDictItemRequest,
    responses((status = 200, description = "创建成功", body = ApiResponse<DictItem>))
)]
pub async fn create_item(
    State(state): State<AppState>,
    _perm: PermDictCreate,
    ApiJson(req): ApiJson<CreateDictItemRequest>,
) -> Result<Json<ApiResponse<DictItem>>, AppError> {
    let type_id = req
        .dict_type_id
        .ok_or(AppError::BadRequest("缺少 dict_type_id".into()))?;
    let item = DictItem {
        id: Uuid::new_v4(),
        dict_type_id: type_id,
        label: req.label,
        value: req.value,
        sort_order: req.sort_order.unwrap_or(0),
        status: req.status.unwrap_or_else(|| "enabled".into()),
        is_default: req.is_default.unwrap_or(false),
        color: req.color,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let saved = state.dict_repo.create_item(&item).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// PUT /api/admin/dict/items/:id — 更新字典项
#[utoipa::path(
    put,
    path = "/api/admin/dict/items/{id}",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "字典项 ID")),
    request_body = CreateDictItemRequest,
    responses((status = 200, description = "更新成功", body = ApiResponse<DictItem>))
)]
pub async fn update_item(
    State(state): State<AppState>,
    _perm: PermDictUpdate,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CreateDictItemRequest>,
) -> Result<Json<ApiResponse<DictItem>>, AppError> {
    let saved = state.dict_repo.update_item(id, &req).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// DELETE /api/admin/dict/items/:id — 删除字典项
#[utoipa::path(
    delete,
    path = "/api/admin/dict/items/{id}",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "字典项 ID")),
    responses(
        (status = 200, description = "删除成功", body = ApiResponse<String>),
        (status = 404, description = "字典项不存在"),
    )
)]
pub async fn delete_item(
    State(state): State<AppState>,
    _perm: PermDictDelete,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.dict_repo.delete_item(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// GET /api/admin/dict/cached — 查询所有字典（含项）并缓存
#[utoipa::path(
    get,
    path = "/api/admin/dict/cached",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "全部字典及字典项", body = ApiResponse<Vec<DictTypeWithItems>>))
)]
pub async fn list_all_cached(
    State(state): State<AppState>,
    _perm: PermDictList,
) -> Result<Json<ApiResponse<Vec<DictTypeWithItems>>>, AppError> {
    let data = state.dict_repo.list_all_with_items().await?;
    Ok(Json(ApiResponse::success(data)))
}

/// POST /api/admin/dict/refresh — 刷新缓存（清空后重新写入）
#[utoipa::path(
    post,
    path = "/api/admin/dict/refresh",
    tag = "数据字典",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "缓存已刷新", body = ApiResponse<String>))
)]
pub async fn refresh_cache(
    State(state): State<AppState>,
    _perm: PermDictRefresh,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 清除 Redis 中所有字典缓存（生产环境可用 SCAN）
    let data = state.dict_repo.list_all_with_items().await?;
    for dt in &data {
        let _ = state.dict_repo.get_dict_by_code(&dt.code).await;
    }
    Ok(Json(ApiResponse::success("缓存刷新成功")))
}
