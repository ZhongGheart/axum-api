//! 两步验证数据模型与 DTO

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── 绑定流程 ─────────────────────────────────────────────────────

/// 2FA 绑定信息（`POST /api/auth/2fa/setup`）
#[derive(Debug, Serialize, ToSchema)]
pub struct TwoFactorSetup {
    /// Base32 形式的密钥，供验证器 App 手动输入
    pub secret: String,
    /// `otpauth://` URI，供扫码
    pub provisioning_uri: String,
}

/// 确认启用 2FA 的请求
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableTwoFactorRequest {
    /// 6 位动态验证码，用来证明 App 侧配置成功
    pub code: String,
}

/// 确认启用 2FA 的响应
#[derive(Debug, Serialize, ToSchema)]
pub struct EnableTwoFactorResponse {
    /// 新生成的恢复码明文——**只在这一次返回，之后不可再取**
    pub recovery_codes: Vec<String>,
}

/// 关闭 2FA 的请求
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DisableTwoFactorRequest {
    /// 当前口令
    ///
    /// **要求口令而不只是"已登录"**：登录态本身可能来自被盗设备，
    /// 让持有会话的人不用再出示第二样东西就能把 2FA 关掉，
    /// 会让整个功能在最需要它的场景下失效。
    pub password: String,
}

/// 重新生成恢复码的响应
#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

/// 当前 2FA 状态（`GET /api/auth/2fa`）
#[derive(Debug, Serialize, ToSchema)]
pub struct TwoFactorStatus {
    /// 是否已启用（登录时会要求第二道因子）
    pub enabled: bool,
    /// 生效时间
    pub enabled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 剩余可用恢复码数量
    pub recovery_codes_remaining: i64,
}

// ── 登录时的二次验证 ────────────────────────────────────────────

/// 二次验证请求
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyTwoFactorRequest {
    /// `POST /api/auth/login` 在需要 2FA 时下发的挑战令牌
    pub challenge_token: String,
    /// 6 位动态验证码，或一个恢复码
    pub code: String,
}
