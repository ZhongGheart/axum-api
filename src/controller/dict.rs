//! 数据字典控制器

use axum::{extract::State, Json};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::permission::{
    PermDictCreate, PermDictDelete, PermDictList, PermDictRefresh, PermDictUpdate,
};
use crate::model::{
    ApiResponse, ChangeType, CreateDictItemRequest, CreateDictTypeRequest, DictCacheRefresh,
    DictItem, DictItemResponse, DictType, DictTypeWithItems, TargetType,
};
use crate::router::AppState;
use crate::utils::api_extractor::{ApiJson, ApiPath};
use crate::utils::audit;

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
    audit: AuditDetail,
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
    audit.push_targeted(
        format!("新建字典类型 \"{}\"（{}）", saved.code, saved.id),
        TargetType::DictType,
        saved.id,
        ChangeType::Create,
        Some(saved.code.clone()),
    );
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CreateDictTypeRequest>,
) -> Result<Json<ApiResponse<DictType>>, AppError> {
    let before = state.dict_repo.find_type_by_id(id).await?;
    let saved = state.dict_repo.update_type(id, &req).await?;
    // `code` 是字典的业务主键：改掉之后前端按 code 取缓存就取到另一份数据，
    // 因此前后两个 code 都要留在审计里
    if before.code != saved.code {
        audit.push_targeted(
            format!(
                "字典类型 \"{}\"（{id}）的 code 改为 \"{}\"",
                before.code, saved.code
            ),
            TargetType::DictType,
            id,
            ChangeType::Update,
            Some(saved.code.clone()),
        );
    } else {
        audit.push_targeted(
            format!("更新字典类型 \"{}\"（{id}）", saved.code),
            TargetType::DictType,
            id,
            ChangeType::Update,
            Some(saved.code.clone()),
        );
    }
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    // 删除会级联清掉该类型下的全部字典项，**每一条都要单独留 target**：
    // 级联删掉的字典项在审计里此前只以"一并删除"四个字存在，
    // 事后无法回答"我用的那个字典值（label=value）是不是被这次删掉了"。
    // 名字也要在删之前取——删掉之后查不到了。
    let label = audit::dict_type_label(&state, id).await;
    // 裸 code 也要在删之前取：删完再查就只剩 None 了。
    // 查失败（不存在）就留空——那一列是锦上添花，摘要文本里仍有名字。
    let type_code = state
        .dict_repo
        .find_type_by_id(id)
        .await
        .ok()
        .map(|t| t.code);
    let cascaded_items = state.dict_repo.list_items(id).await.unwrap_or_default();
    state.dict_repo.delete_type(id).await?;
    audit.push_targeted(
        format!("删除{label}（{id}），其下字典项一并删除"),
        TargetType::DictType,
        id,
        ChangeType::Delete,
        // 存裸 code 而不是 `label`（后者形如 `字典类型 "xxx"`）：
        // 这一列在所有行里都是裸名字，带前缀会与其它列不一致
        type_code,
    );
    for item in cascaded_items {
        audit.add_target(
            TargetType::DictItem,
            item.id,
            ChangeType::Delete,
            Some(format!("{}={}", item.label, item.value)),
        );
    }
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
    audit: AuditDetail,
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
    audit.push_targeted(
        format!(
            "新建字典项 \"{}={}\"（{}），属于字典类型 {}",
            saved.label, saved.value, saved.id, saved.dict_type_id
        ),
        TargetType::DictItem,
        saved.id,
        ChangeType::Create,
        Some(format!("{}={}", saved.label, saved.value)),
    );
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CreateDictItemRequest>,
) -> Result<Json<ApiResponse<DictItem>>, AppError> {
    let before = state.dict_repo.find_item_by_id(id).await?;
    let saved = state.dict_repo.update_item(id, &req).await?;
    audit.push_targeted(
        format!(
            "字典项（{id}）由 \"{}={}\" 改为 \"{}={}\"",
            before.label, before.value, saved.label, saved.value
        ),
        TargetType::DictItem,
        id,
        ChangeType::Update,
        Some(format!("{}={}", saved.label, saved.value)),
    );
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
    audit: AuditDetail,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    let label = audit::dict_item_label(&state, id).await;
    // 裸 `label=value` 在删之前取，理由同 `delete_type`
    let item_label = state
        .dict_repo
        .find_item_by_id(id)
        .await
        .ok()
        .map(|i| format!("{}={}", i.label, i.value));
    state.dict_repo.delete_item(id).await?;
    audit.push_targeted(
        format!("删除{label}（{id}）"),
        TargetType::DictItem,
        id,
        ChangeType::Delete,
        // 字典项行已消失，这一列是"删掉的是哪个值"的最后存档
        item_label,
    );
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
    responses((status = 200, description = "缓存已刷新（返回真实的清理数量）", body = ApiResponse<DictCacheRefresh>))
)]
pub async fn refresh_cache(
    State(state): State<AppState>,
    _perm: PermDictRefresh,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<DictCacheRefresh>>, AppError> {
    // **先真删，再回填。**
    //
    // 此前的实现是 `for dt in &data { get_dict_by_code(&dt.code).await }`，
    // 而 `get_dict_by_code` 第一件事就是读缓存、命中即返回——于是这个循环
    // 只是把同一份陈旧数据读出来再原样写回去。注释写的"清除 Redis 中所有
    // 字典缓存"从未发生，界面却弹"缓存刷新成功"，审计也记"刷新字典缓存"。
    //
    // 它坏在唯一该起作用的时候：写路径的 `invalidate_cache` 失败之后，
    // 管理员能点的就只剩这个按钮。
    let cleared_keys = state.dict_repo.clear_all_dict_cache().await?;

    let types = state.dict_repo.list_types().await?;
    let mut reloaded_types = 0u64;
    let mut skipped_disabled_types = 0u64;
    for t in &types {
        // 禁用类型不进读取端点，回填它的缓存没有任何读者。
        // 如实计入"跳过"而不是悄悄算进成功里。
        if t.status != "enabled" {
            skipped_disabled_types += 1;
            continue;
        }
        state.dict_repo.get_dict_by_code(&t.code).await?;
        reloaded_types += 1;
    }

    // **不声明任何结构化 target**：这个端点只清 Redis 缓存并回填，
    // 没有改动任何一条持久化的字典类型 / 字典项。硬给它挂一个 target
    // 会让"这个字典项被改过"出现一条不成立的记录——
    // 而缓存刷新与字典内容变更在事后追溯里是两回事。
    audit.push(format!(
        "清空字典缓存 {cleared_keys} 个键，回填 {reloaded_types} 个类型（跳过 {skipped_disabled_types} 个已禁用类型）"
    ));
    Ok(Json(ApiResponse::success(DictCacheRefresh {
        cleared_keys,
        reloaded_types,
        skipped_disabled_types,
    })))
}
