//! 两步验证控制器
//!
//! 自助端点（绑定的都是**当前登录用户自己的** 2FA，因此一律挂在
//! `auth_middleware` 下并取 `auth_user.user_id`，不接受任何"操作某个用户"的入参）。
//!
//! 唯一的公开端点是 `POST /api/auth/2fa/verify`：它持有挑战令牌，
//! 而挑战令牌本身就代表"口令已校验通过"。

use axum::{extract::State, Json};

use crate::error::AppError;
use crate::middleware::audit_log::AuditDetail;
use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::client_ip::ClientIp;
use crate::model::{
    ApiResponse, ChangeType, DisableTwoFactorRequest, EnableTwoFactorRequest, LoginResponse,
    RecoveryCodesResponse, TargetType, TwoFactorSetup, TwoFactorStatus, VerifyTwoFactorRequest,
};
use crate::router::AppState;
use crate::utils::api_extractor::ApiJson;

/// GET /api/auth/2fa — 查询当前用户的 2FA 状态
#[utoipa::path(
    get,
    path = "/api/auth/2fa",
    tag = "两步验证",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前 2FA 状态", body = ApiResponse<TwoFactorStatus>))
)]
pub async fn status(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<TwoFactorStatus>>, AppError> {
    let status = state.two_factor_service.status(auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(status)))
}

/// POST /api/auth/2fa/setup — 开始绑定，返回密钥与扫码 URI
#[utoipa::path(
    post,
    path = "/api/auth/2fa/setup",
    tag = "两步验证",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "密钥已生成（尚未生效，需再调 enable 确认）", body = ApiResponse<TwoFactorSetup>),
        (status = 400, description = "已启用 2FA，需先关闭再重新绑定"),
    )
)]
pub async fn setup(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<TwoFactorSetup>>, AppError> {
    let user = state
        .auth_service
        .user_repo
        .find_by_id(auth_user.user_id)
        .await?;
    let setup = state
        .two_factor_service
        .setup(&state.redis_client, auth_user.user_id, &user.username)
        .await?;
    audit.push_targeted(
        "开始绑定两步验证：生成新密钥（尚未生效）".to_string(),
        TargetType::UserTwoFactor,
        auth_user.user_id,
        // 此刻密钥还没生效，说它"已启用"是谎报。`create` 只表示
        // "为这个账号新建了一条两步验证配置"，不声称 2FA 已经在起作用。
        ChangeType::Create,
        Some(auth_user.username.clone()),
    );
    Ok(Json(ApiResponse::success(setup)))
}

/// POST /api/auth/2fa/enable — 用 App 生成的码确认启用，返回恢复码
#[utoipa::path(
    post,
    path = "/api/auth/2fa/enable",
    tag = "两步验证",
    security(("bearer_auth" = [])),
    request_body = EnableTwoFactorRequest,
    responses(
        (status = 200, description = "已启用，返回一次性恢复码", body = ApiResponse<RecoveryCodesResponse>),
        (status = 400, description = "验证码错误，或绑定流程已超时"),
    )
)]
pub async fn enable(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
    ApiJson(req): ApiJson<EnableTwoFactorRequest>,
) -> Result<Json<ApiResponse<RecoveryCodesResponse>>, AppError> {
    let resp = state
        .two_factor_service
        .enable(&state.redis_client, auth_user.user_id, &req.code)
        .await?;
    audit.push_targeted(
        "启用两步验证（已生成恢复码）".to_string(),
        TargetType::UserTwoFactor,
        auth_user.user_id,
        ChangeType::Enable,
        Some(auth_user.username.clone()),
    );
    Ok(Json(ApiResponse::success(RecoveryCodesResponse {
        recovery_codes: resp.recovery_codes,
    })))
}

/// POST /api/auth/2fa/disable — 关闭 2FA（需出示当前口令）
#[utoipa::path(
    post,
    path = "/api/auth/2fa/disable",
    tag = "两步验证",
    security(("bearer_auth" = [])),
    request_body = DisableTwoFactorRequest,
    responses(
        (status = 200, description = "已关闭", body = ApiResponse<String>),
        (status = 400, description = "口令不正确，或未启用 2FA"),
    )
)]
pub async fn disable(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
    ApiJson(req): ApiJson<DisableTwoFactorRequest>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    state
        .two_factor_service
        .disable(auth_user.user_id, &state.auth_service.user_repo, &req)
        .await?;
    audit.push_targeted(
        "关闭两步验证（已出示当前口令）".to_string(),
        TargetType::UserTwoFactor,
        auth_user.user_id,
        ChangeType::Disable,
        Some(auth_user.username.clone()),
    );
    Ok(Json(ApiResponse::success("两步验证已关闭".to_string())))
}

/// POST /api/auth/2fa/recovery-codes — 重新生成恢复码
#[utoipa::path(
    post,
    path = "/api/auth/2fa/recovery-codes",
    tag = "两步验证",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "已生成新的一批恢复码，旧码作废", body = ApiResponse<RecoveryCodesResponse>),
        (status = 400, description = "未启用 2FA"),
    )
)]
pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    audit: AuditDetail,
) -> Result<Json<ApiResponse<RecoveryCodesResponse>>, AppError> {
    let resp = state
        .two_factor_service
        .regenerate_recovery_codes(auth_user.user_id)
        .await?;
    audit.push_targeted(
        "重新生成两步验证恢复码（旧码已作废）".to_string(),
        TargetType::UserTwoFactor,
        auth_user.user_id,
        ChangeType::Update,
        Some(auth_user.username.clone()),
    );
    Ok(Json(ApiResponse::success(resp)))
}

/// POST /api/auth/2fa/verify — 登录第二步：校验第二道因子并换取令牌
#[utoipa::path(
    post,
    path = "/api/auth/2fa/verify",
    tag = "两步验证",
    request_body = VerifyTwoFactorRequest,
    responses(
        (status = 200, description = "验证通过，返回访问令牌", body = ApiResponse<LoginResponse>),
        (status = 400, description = "验证码错误、恢复码已用过，或挑战令牌已失效"),
        (status = 429, description = "二次验证失败次数过多"),
    )
)]
pub async fn verify(
    State(state): State<AppState>,
    client_ip: ClientIp,
    ApiJson(req): ApiJson<VerifyTwoFactorRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let resp = state
        .auth_service
        .complete_two_factor_login(
            &state.redis_client,
            &req.challenge_token,
            &req.code,
            &client_ip.0,
        )
        .await?;
    Ok(Json(ApiResponse::success(resp)))
}
