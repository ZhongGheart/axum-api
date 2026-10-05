//! 系统参数控制器

use axum::{extract::Path, extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::permission::{PermSettingList, PermSettingUpdate};
use crate::model::{ApiResponse, ChangeType, TargetType};
use crate::repository::setting::SettingView;
use crate::router::AppState;
use crate::utils::api_extractor::ApiJson;

/// 修改单个参数的请求体
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSettingRequest {
    /// 取值；以文本表示，实际类型由服务端按参数定义解析
    pub value: String,
}

/// 面向未登录页面的口令策略（注册/改密页显示"需要满足什么"）
///
/// **刻意不含** `expiry_days`、`max_failures` 这类参数。
/// 它们对访客没有价值，而把"账号多久被锁一次"暴露给未登录端点
/// 等于给爆破者提供了可直接调的参数面板。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PublicPasswordPolicy {
    /// 最小长度
    pub min_length: usize,
    /// 最大长度
    pub max_length: usize,
    /// 最少字符类别数
    pub min_char_classes: usize,
    /// 是否强制大小写混合
    pub require_mixed_case: bool,
}

/// GET /api/admin/settings — 列出全部系统参数
#[utoipa::path(
    get,
    path = "/api/admin/settings",
    tag = "系统参数",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "系统参数列表", body = ApiResponse<Vec<SettingView>>))
)]
pub async fn list_settings(
    State(state): State<AppState>,
    _perm: PermSettingList,
) -> Result<Json<ApiResponse<Vec<SettingView>>>, AppError> {
    let list = state.setting_service.list().await?;
    Ok(Json(ApiResponse::success(list)))
}

/// PUT /api/admin/settings/{key} — 修改单个参数
#[utoipa::path(
    put,
    path = "/api/admin/settings/{key}",
    tag = "系统参数",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "参数名")),
    request_body = UpdateSettingRequest,
    responses(
        (status = 200, description = "修改成功", body = ApiResponse<SettingView>),
        (status = 400, description = "参数不存在、取值超出范围，或与另一参数冲突"),
    )
)]
pub async fn update_setting(
    State(state): State<AppState>,
    _perm: PermSettingUpdate,
    audit: AuditDetail,
    user: AuthenticatedUser,
    Path(key): Path<String>,
    ApiJson(req): ApiJson<UpdateSettingRequest>,
) -> Result<Json<ApiResponse<SettingView>>, AppError> {
    // 先读旧值：**审计必须记下"从什么改成什么"**。
    // 只记新值的话，"谁把口令最小长度从 8 调到 20"在事后无法回答，
    // 而这正是出事时第一个要查的问题。
    let before = state
        .setting_service
        .list()
        .await?
        .into_iter()
        .find(|v| v.key == key);
    let old_value = before
        .as_ref()
        .map(|v| v.value.clone())
        .unwrap_or_else(|| "(未设置)".into());

    state
        .setting_service
        .update(&key, &req.value, user.user_id)
        .await?;

    audit.push_key_targeted(
        format!("修改系统参数 {key}：{old_value} → {}", req.value.trim()),
        TargetType::Setting,
        &key,
        ChangeType::Update,
        // label 存参数名本身：参数行还在，但"改之前叫什么"没有意义，
        // 参数名是它的身份
        Some(key.to_string()),
    );

    let saved = state
        .setting_service
        .list()
        .await?
        .into_iter()
        .find(|v| v.key == key)
        .ok_or_else(|| AppError::NotFound(format!("没有名为 {key} 的系统参数")))?;
    Ok(Json(ApiResponse::success(saved)))
}

/// POST /api/admin/settings/{key}/reset — 把参数复位成默认值
#[utoipa::path(
    post,
    path = "/api/admin/settings/{key}/reset",
    tag = "系统参数",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "参数名")),
    responses(
        (status = 200, description = "已复位为默认值", body = ApiResponse<SettingView>),
        (status = 400, description = "参数不存在"),
    )
)]
pub async fn reset_setting(
    State(state): State<AppState>,
    _perm: PermSettingUpdate,
    audit: AuditDetail,
    user: AuthenticatedUser,
    Path(key): Path<String>,
) -> Result<Json<ApiResponse<SettingView>>, AppError> {
    let def = crate::model::setting::find_def(&key)
        .ok_or_else(|| AppError::BadRequest(format!("没有名为 {key} 的系统参数")))?;

    state.setting_service.reset(&key, user.user_id).await?;
    audit.push_key_targeted(
        format!("复位系统参数 {key} 为默认值 {}", def.default),
        TargetType::Setting,
        &key,
        ChangeType::Update,
        Some(key.to_string()),
    );

    let saved = state
        .setting_service
        .list()
        .await?
        .into_iter()
        .find(|v| v.key == key)
        .ok_or_else(|| AppError::NotFound(format!("没有名为 {key} 的系统参数")))?;
    Ok(Json(ApiResponse::success(saved)))
}

/// POST /api/admin/settings/refresh-cache — 清理参数缓存
#[utoipa::path(
    post,
    path = "/api/admin/settings/refresh-cache",
    tag = "系统参数",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "缓存已清理"))
)]
pub async fn refresh_cache(
    State(state): State<AppState>,
    _perm: PermSettingUpdate,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<String>>, AppError> {
    // 这不是摆设：v0.16.0 的"刷新字典缓存"就是**唯一一个**点了没反应的按钮
    //（返回成功、Redis 里的键没动、读到的还是陈旧数据）。
    // 这里真的删掉缓存键，下一次读会回源查库。
    state.setting_service.repo().invalidate_cache().await?;
    // **不声明 target**：只清 Redis 缓存，没有改动任何一条参数。
    // 挂一个 `setting/update` 会让"这个参数被人改过"出现不成立的记录。
    audit.push("清理系统参数缓存".to_string());
    Ok(Json(ApiResponse::success("参数缓存已清理".to_string())))
}

/// GET /api/settings/password-policy — 面向登录/注册页的口令策略
#[utoipa::path(
    get,
    path = "/api/settings/password-policy",
    tag = "系统参数",
    responses((status = 200, description = "当前口令策略", body = ApiResponse<PublicPasswordPolicy>))
)]
pub async fn public_password_policy(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<PublicPasswordPolicy>>, AppError> {
    // **公开端点**（挂在受保护路由之外）：注册页是未登录状态，
    // 没有它的话用户只能在提交失败后从报错里猜出口令要求。
    //
    // 只暴露四条"用户自己设口令时必须知道"的规则，
    // 不含锁定阈值与有效期——见 [`PublicPasswordPolicy`] 的注释。
    let p = state.setting_service.password_policy().await;
    Ok(Json(ApiResponse::success(PublicPasswordPolicy {
        min_length: p.min_length,
        max_length: p.max_length,
        min_char_classes: p.min_char_classes,
        require_mixed_case: p.require_mixed_case,
    })))
}
