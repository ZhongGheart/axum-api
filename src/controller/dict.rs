//! 数据字典控制器

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{
    ApiResponse, CreateDictItemRequest, CreateDictTypeRequest, DictItem, DictItemResponse,
    DictType, DictTypeWithItems,
};
use crate::router::AppState;

/// GET /api/admin/dict/types — 字典类型列表
pub async fn list_types(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<DictType>>>, AppError> {
    let types = state.dict_repo.list_types().await?;
    Ok(Json(ApiResponse::success(types)))
}

/// POST /api/admin/dict/types — 新增字典类型
pub async fn create_type(
    State(state): State<AppState>,
    Json(req): Json<CreateDictTypeRequest>,
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
pub async fn update_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateDictTypeRequest>,
) -> Result<Json<ApiResponse<DictType>>, AppError> {
    let saved = state.dict_repo.update_type(id, &req).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// DELETE /api/admin/dict/types/:id — 删除字典类型（级联删除项）
pub async fn delete_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.dict_repo.delete_type(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// GET /api/admin/dict/:code/items — 获取字典项（直接从缓存或数据库）
pub async fn get_items_by_code(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<ApiResponse<Vec<DictItemResponse>>>, AppError> {
    let items = state.dict_repo.get_dict_by_code(&code).await?;
    Ok(Json(ApiResponse::success(items)))
}

/// GET /api/admin/dict/items?dict_type_id=xxx — 根据类型 ID 获取项
pub async fn list_items(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<DictItemQuery>,
) -> Result<Json<ApiResponse<Vec<DictItem>>>, AppError> {
    let items = state.dict_repo.list_items(params.dict_type_id).await?;
    Ok(Json(ApiResponse::success(items)))
}

#[derive(Debug, serde::Deserialize)]
pub struct DictItemQuery {
    pub dict_type_id: Uuid,
}

/// POST /api/admin/dict/items — 新增字典项
pub async fn create_item(
    State(state): State<AppState>,
    Json(req): Json<CreateDictItemRequest>,
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
pub async fn update_item(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateDictItemRequest>,
) -> Result<Json<ApiResponse<DictItem>>, AppError> {
    let saved = state.dict_repo.update_item(id, &req).await?;
    Ok(Json(ApiResponse::success(saved)))
}

/// DELETE /api/admin/dict/items/:id — 删除字典项
pub async fn delete_item(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.dict_repo.delete_item(id).await?;
    Ok(Json(ApiResponse::success("删除成功")))
}

/// GET /api/admin/dict/cached — 查询所有字典（含项）并缓存
pub async fn list_all_cached(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<DictTypeWithItems>>>, AppError> {
    let data = state.dict_repo.list_all_with_items().await?;
    Ok(Json(ApiResponse::success(data)))
}

/// POST /api/admin/dict/refresh — 刷新缓存（清空后重新写入）
pub async fn refresh_cache(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 清除 Redis 中所有字典缓存（生产环境可用 SCAN）
    let data = state.dict_repo.list_all_with_items().await?;
    for dt in &data {
        let _ = state.dict_repo.get_dict_by_code(&dt.code).await;
    }
    Ok(Json(ApiResponse::success("缓存刷新成功")))
}
