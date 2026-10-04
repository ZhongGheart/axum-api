//! 认证控制器
//!
//! 处理认证相关的 HTTP 请求，包括注册、登录、获取当前用户信息、登出。

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::client_ip::ClientIp;
use crate::model::{
    ApiResponse, ChangePasswordRequest, LoginRequest, LoginResponse, RegisterRequest,
    UpdateProfileRequest, UserInfo,
};
use crate::router::AppState;
use crate::utils::api_extractor::ApiJson;

/// POST /api/auth/register — 用户注册
#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "认证",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "注册成功", body = ApiResponse<UserInfo>),
        (status = 400, description = "参数不合法"),
        (status = 409, description = "用户名或邮箱已存在"),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    client_ip: ClientIp,
    ApiJson(req): ApiJson<RegisterRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user_info = state.auth_service.register(req, &client_ip.0).await?;
    Ok(Json(ApiResponse::success(user_info)))
}

/// POST /api/auth/login — 用户登录
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "认证",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = ApiResponse<LoginResponse>),
        (status = 401, description = "用户名或密码错误"),
        (status = 429, description = "登录失败次数过多，已临时锁定"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    client_ip: ClientIp,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let login_resp = state
        .auth_service
        .login(req, &state.redis_client, &client_ip.0)
        .await?;
    Ok(Json(ApiResponse::success(login_resp)))
}

/// GET /api/auth/me — 获取当前用户信息（含角色列表）
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户信息（含角色列表）", body = ApiResponse<UserInfo>))
)]
pub async fn me(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user = state
        .auth_service
        .user_repo
        .find_by_id(auth_user.user_id)
        .await?;
    let roles = state
        .auth_service
        .role_repo
        .find_roles_by_user_id(auth_user.user_id)
        .await?;
    let user_info = crate::model::UserInfo::new(user, roles);
    Ok(Json(ApiResponse::success(user_info)))
}

/// POST /api/auth/logout — 用户登出（仅注销当前令牌）
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "认证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "登出成功（仅当前令牌失效）", body = ApiResponse<String>))
)]
pub async fn logout(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state
        .auth_service
        .logout(
            &state.redis_client,
            auth_user.user_id,
            &auth_user.token_jti,
            auth_user.token_exp,
        )
        .await?;
    audit.push(format!(
        "登出：注销当前令牌（仅本次会话，其余并发会话不受影响），账号 \"{}\"",
        auth_user.username
    ));
    Ok(Json(ApiResponse::success("登出成功")))
}

/// PUT /api/auth/password — 自助修改口令
#[utoipa::path(
    put,
    path = "/api/auth/password",
    tag = "认证",
    security(("bearer_auth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "改密成功，该用户全部会话已失效（需重新登录）", body = ApiResponse<String>),
        (status = 400, description = "当前密码不正确，或新密码不满足复杂度策略"),
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
    ApiJson(req): ApiJson<ChangePasswordRequest>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state
        .auth_service
        .change_password(
            &state.redis_client,
            auth_user.user_id,
            &req.old_password,
            &req.new_password,
        )
        .await?;
    // 记"改了什么"而不记新旧口令：旧口令本身是当前的凭据，
    // 新口令是改完后的凭据，两者写进长期表都是把当前有效的秘密再复制一份
    audit.push(format!(
        "自助修改口令并吊销该账号全部会话，账号 \"{}\"",
        auth_user.username
    ));
    Ok(Json(ApiResponse::success("密码修改成功，请重新登录")))
}

/// PUT /api/auth/profile — 自助修改资料（展示名 / 头像）
///
/// 与 `PUT /api/auth/password` 并列的第二个自助端点。**不含邮箱**：
/// 见迁移 014 注释——没有邮件通道就无法证明新邮箱归提交者所有。
#[utoipa::path(
    put,
    path = "/api/auth/profile",
    tag = "认证",
    security(("bearer_auth" = [])),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "资料已更新（返回归一后的完整用户信息）", body = ApiResponse<UserInfo>),
        (status = 400, description = "展示名超长，或头像路径不是站内相对路径"),
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
    ApiJson(req): ApiJson<UpdateProfileRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    // 只把**请求里真的带了**的字段交给仓储。三态在反序列化器里已经分好：
    // 不带 = None（不改），带了但为 null/空串 = Some(None)（清空），
    // 带了值 = Some(Some(v))（设置）。这里只做 &str 借用转换。
    let info = state
        .auth_service
        .update_profile(
            auth_user.user_id,
            req.display_name.as_ref().map(|v| v.as_deref()),
            req.avatar_url.as_ref().map(|v| v.as_deref()),
        )
        .await?;

    // 记"改了哪些字段"而不记新旧值全文。展示名不是凭据，
    // 但把整段内容写进长期表没有额外价值，超长内容还会让审计表迅速膨胀。
    let set_display_name = matches!(req.display_name.as_ref(), Some(Some(_)));
    let set_avatar = matches!(req.avatar_url.as_ref(), Some(Some(_)));

    let mut changes: Vec<&str> = Vec::new();
    // 区分"设置为新值"与"清空"：审计里都写"改了展示名"会让人以为设了值，
    // 而清空恰恰是最需要留痕的那一种（它让列表页的显示退回用户名）。
    if req.display_name.is_some() {
        changes.push(if set_display_name {
            "展示名"
        } else {
            "展示名（已清空）"
        });
    }
    if req.avatar_url.is_some() {
        changes.push(if set_avatar {
            "头像"
        } else {
            "头像（已清空）"
        });
    }
    // 先绑定再进 format!：`changes.join()` 是临时值，
    // 直接写在 format! 参数里会在语句结束时就被释放（E0716）。
    let changed = if changes.is_empty() {
        "无字段变更".to_string()
    } else {
        changes.join("、")
    };
    audit.push(format!(
        "自助修改资料（{changed}），账号 \"{}\"",
        auth_user.username
    ));

    Ok(Json(ApiResponse::success(info)))
}

/// 上传头像后的响应体
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AvatarUploadResponse {
    /// 可直接写进 `avatar_url` 的站内相对路径
    pub url: String,
}

/// POST /api/auth/profile/avatar — 上传并设置当前用户的头像
///
/// ── 为什么上传端点自己就把头像设上，而不是"上传后返回路径让前端再 PUT 一次" ──
///
/// 分成两步的话，中间那一步失败（用户上传完就关掉页面）会留下一个
/// 谁也引用不到的孤儿文件；而一个用户能"上传但设不上"的中间态本身没有意义。
/// 上传与设置在同一个事务序列里完成，失败时把刚写的文件删掉。
#[utoipa::path(
    post,
    path = "/api/auth/profile/avatar",
    tag = "认证",
    security(("bearer_auth" = [])),
    request_body(content = String, content_type = "multipart/form-data",
        description = "`file` 字段：图片文件，字段名必须是 `file`"),
    responses(
        (status = 200, description = "头像已上传并写入当前用户", body = ApiResponse<AvatarUploadResponse>),
        (status = 400, description = "缺少 file 字段，或类型不在白名单内"),
        (status = 413, description = "图片超过 UPLOAD_MAX_FILE_SIZE"),
    )
)]
pub async fn upload_avatar(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
    // 接 `Result<Multipart, MultipartRejection>` 而不是直接接 `Multipart`：
    // 直接接的话，Content-Type 不对时提取器**在进入处理函数之前**就返回
    // `text/plain` 的 400，绕过统一错误信封——同一套入参错误于是有两种形态，
    // 调用方没法只靠 `code` 分支处理。
    multipart: Result<axum::extract::Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<ApiResponse<AvatarUploadResponse>>, AppError> {
    let cfg = state.upload_config.clone();

    let mut multipart = multipart
        .map_err(|e| AppError::BadRequest(format!("请求必须是 multipart/form-data: {e}")))?;

    // 一个请求里只认第一个 `file` 字段。多字段会让"到底用哪个"变得不确定，
    // 而那正是可以拿大文件挤占磁盘的地方。
    // 循环体里就把字段读完再取下一个：`Field` 持有对 `multipart` 的可变借用，
    // 若把它存到循环外再继续迭代，编译器会以"可变借用跨越下一次迭代"拒绝。
    let mut found: Option<(String, axum::body::Bytes)> = None;
    loop {
        let next = multipart
            .next_field()
            .await
            .map_err(|e| AppError::BadRequest(format!("解析上传内容失败: {e}")))?;
        let Some(field) = next else { break };
        if field.name() != Some("file") {
            continue;
        }
        let mime = field
            .content_type()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        // 类型先判再读内容：不合规的请求不该有机会把字节流拖进来
        if !cfg.allowed_mime_types.contains(&mime) {
            return Err(AppError::BadRequest(format!(
                "不支持的图片类型：{mime}（仅接受 JPEG / PNG / WebP / GIF）"
            )));
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取上传内容失败: {e}")))?;
        found = Some((mime, bytes));
        break;
    }

    let (mime, bytes) = found.ok_or_else(|| AppError::BadRequest("缺少 file 字段".into()))?;
    if bytes.len() > cfg.max_file_size {
        return Err(AppError::PayloadTooLarge(format!(
            "图片不超过 {} 字节，当前 {}",
            cfg.max_file_size,
            bytes.len()
        )));
    }

    let saved = crate::utils::upload::save_avatar_bytes(&cfg, &mime, &bytes).await?;

    // 旧头像必须在**写入新值之前**读出来：写完之后库里已经指向新文件，
    // 回头再读只能读到新路径，"替换掉旧文件"这件事就悄悄失效了。
    let old_avatar = state
        .auth_service
        .user_repo
        .find_by_id(auth_user.user_id)
        .await?
        .avatar_url
        .clone();

    // 设置失败要把刚写的文件删掉：否则每次失败都留下一个没人引用的孤儿文件，
    // 而用户会以为"至少图还在"。清理失败只记日志，不覆盖原始错误——
    // 让调用方看到"数据库写失败"比看到"清理也失败了"有用得多。
    if let Err(e) = state
        .auth_service
        .update_profile(auth_user.user_id, None, Some(Some(&saved.url)))
        .await
    {
        if let Err(cleanup_err) =
            crate::utils::upload::delete_uploaded_avatar(&cfg, &saved.url).await
        {
            tracing::warn!("头像写入用户失败后清理文件也失败了: {cleanup_err}");
        }
        return Err(e);
    }

    // 替换掉旧头像：不清的话磁盘上会累积用户的每一版头像，
    // 而界面上永远只显示最新那一版。
    if let Some(old) = old_avatar.as_deref() {
        if old != saved.url {
            if let Err(e) = crate::utils::upload::delete_uploaded_avatar(&cfg, old).await {
                tracing::warn!("删除旧头像失败（不影响本次上传结果）: {e}");
            }
        }
    }

    audit.push(format!(
        "自助上传头像（{}，{} 字节），账号 \"{}\"",
        saved.url,
        bytes.len(),
        auth_user.username
    ));

    Ok(Json(ApiResponse::success(AvatarUploadResponse {
        url: saved.url,
    })))
}

/// 健康检查响应体
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthPayload {
    /// 总体状态：`ok` / `degraded`
    pub status: &'static str,
    /// 数据库连通性：`up` / `down`
    pub database: &'static str,
    /// Redis 连通性：`up` / `down`
    pub redis: &'static str,
}

/// GET /api/health — 健康检查
///
/// 真实探测数据库与 Redis：任一依赖不可用时返回 503，
/// 以便容器编排与负载均衡摘除该实例。
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "系统",
    responses(
        (status = 200, description = "服务正常", body = ApiResponse<HealthPayload>),
        (status = 503, description = "依赖服务不可用", body = ApiResponse<HealthPayload>),
    )
)]
pub async fn health(State(state): State<AppState>) -> axum::response::Response {
    let database_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.db_pool.writer())
        .await
        .is_ok();
    let redis_ok = state.redis_client.ping().await.is_ok();
    let healthy = database_ok && redis_ok;

    let payload = HealthPayload {
        status: if healthy { "ok" } else { "degraded" },
        database: if database_ok { "up" } else { "down" },
        redis: if redis_ok { "up" } else { "down" },
    };

    let (status, message) = if healthy {
        (StatusCode::OK, "服务运行正常")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "依赖服务不可用")
    };

    (
        status,
        Json(ApiResponse {
            code: status.as_u16(),
            message: message.to_string(),
            data: Some(payload),
        }),
    )
        .into_response()
}
