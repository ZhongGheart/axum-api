//! 两步验证服务层
//!
//! 职责：绑定、关闭、换恢复码，以及登录时的二次校验。
//!
//! ── 绑定为什么是"两步"而不是"一步" ─────────────────────────
//! `setup` 只把密钥放进 **Redis**（15 分钟 TTL），不落库；
//! 只有 `enable` 拿用户提交的验证码比对通过后才写进数据库。
//! 因此"密钥已落库但用户从没验证过 App 能用"这个状态不存在——
//! 用户扫码后不输码就离开，下次重来一遍即可，不会留下垃圾数据。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::two_factor::{
    DisableTwoFactorRequest, EnableTwoFactorResponse, RecoveryCodesResponse, TwoFactorSetup,
    TwoFactorStatus,
};
use crate::repository::two_factor::TwoFactorRepository;
use crate::utils::redis::RedisClient;
use crate::utils::totp;

/// 待确认密钥的 Redis 前缀
const PENDING_SECRET_PREFIX: &str = "2fa:pending:";
/// 登录挑战令牌的 Redis 前缀
const CHALLENGE_PREFIX: &str = "2fa:challenge:";
/// 二次验证失败计数的 scope 前缀（复用登录失败计数的存储与阈值）
const FAIL_SCOPE_PREFIX: &str = "2fa-user:";

/// 挑战令牌里携带的登录上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLogin {
    /// 已通过口令校验的用户
    pub user_id: Uuid,
    /// 口令失败计数用的账号 scope
    ///
    /// **必须随挑战一起带走**：清零失败计数要清的正是"这次登录"用的那两个 key，
    /// 而它们由用户**当时输入的字符串**归一而来（`Admin` 与 `admin` 是两个不同的 key）。
    /// 到第二步再从库里取 `users.username` 去清，很可能清不到用户当时输的那个。
    pub account_scope: String,
    /// 口令失败计数用的 IP scope
    pub ip_scope: String,
}

/// 待确认密钥的存活时间：15 分钟
const PENDING_TTL_SECONDS: u64 = 15 * 60;

/// 挑战令牌存活时间：5 分钟
///
/// 它代表"口令已验证通过"这个中间结论，必须短命。
const CHALLENGE_TTL_SECONDS: u64 = 5 * 60;

/// 验证器 App 里显示的签发方名称
const ISSUER: &str = "Axum Admin";

/// "未启用 2FA"的统一提示
const NOT_ENABLED: &str = "两步验证未启用";

/// 挑战令牌失效 / 2FA 状态已变的统一提示
const CHALLENGE_EXPIRED: &str = "两步验证状态已变更，请重新登录";

/// 两步验证服务
#[derive(Debug, Clone)]
pub struct TwoFactorService {
    repo: TwoFactorRepository,
    /// TOTP 密钥的加密密钥
    encryption_key: String,
    /// 系统参数服务：二次验证的失败阈值与窗口**复用**登录爆破防护那套参数
    setting_service: crate::service::setting::SettingService,
}

impl TwoFactorService {
    pub fn new(
        repo: TwoFactorRepository,
        encryption_key: String,
        setting_service: crate::service::setting::SettingService,
    ) -> Self {
        Self {
            repo,
            encryption_key,
            setting_service,
        }
    }

    pub fn repo(&self) -> &TwoFactorRepository {
        &self.repo
    }

    /// 当前 2FA 状态
    pub async fn status(&self, user_id: Uuid) -> Result<TwoFactorStatus, AppError> {
        let state = self.repo.find_state(user_id).await?;
        let remaining = if state.is_enabled() {
            self.repo.count_unused_recovery_codes(user_id).await?
        } else {
            0
        };
        Ok(TwoFactorStatus {
            enabled: state.is_enabled(),
            enabled_at: state.enabled_at,
            recovery_codes_remaining: remaining,
        })
    }

    /// 是否处于"登录必须过第二道因子"的状态
    pub async fn is_enabled(&self, user_id: Uuid) -> Result<bool, AppError> {
        Ok(self.repo.find_state(user_id).await?.is_enabled())
    }

    /// 开始绑定：生成密钥并暂存，等 `enable` 确认
    pub async fn setup(
        &self,
        redis: &RedisClient,
        user_id: Uuid,
        username: &str,
    ) -> Result<TwoFactorSetup, AppError> {
        // 已启用时不允许直接重绑：那会让用户"再扫一次就换密钥"，
        // 一次误操作就把正在用的密钥作废。必须先 `disable`。
        if self.is_enabled(user_id).await? {
            return Err(AppError::ValidationFailed(
                "两步验证已启用，请先关闭后再重新绑定".to_string(),
            ));
        }
        let secret = totp::generate_secret();
        // 存的是**加密后**的密钥：Redis 里也不留明文。
        // Redis 常被当作"内部网络就安全"，但它同样会有 RDB/AOF 持久化与只读副本。
        let blob = totp::encrypt_secret(&secret, &self.encryption_key)?;
        redis
            .set_string(
                &format!("{PENDING_SECRET_PREFIX}{user_id}"),
                &URL_SAFE_NO_PAD.encode(&blob),
                PENDING_TTL_SECONDS,
            )
            .await?;
        Ok(TwoFactorSetup {
            provisioning_uri: totp::provisioning_uri(&secret, username, ISSUER),
            secret,
        })
    }

    /// 确认启用：校验用户提交的验证码，通过后密钥落库并生成恢复码
    pub async fn enable(
        &self,
        redis: &RedisClient,
        user_id: Uuid,
        code: &str,
    ) -> Result<EnableTwoFactorResponse, AppError> {
        let key = format!("{PENDING_SECRET_PREFIX}{user_id}");
        // 先**读**不删：验证码输错时用户应当能就地重试。
        // 早先这里用的是 `take_string`（读并删），于是输错一次就把待确认密钥
        // 一起删掉了——用户必须重新走一遍"生成密钥 → 重新扫码"，
        // 而他刚扫码装好的 App 里还留着上一把已经作废的密钥。
        let pending = redis.get_string(&key).await?.ok_or_else(|| {
            AppError::ValidationFailed("绑定已超时，请重新开始绑定流程".to_string())
        })?;
        // 解不开说明缓存被换成了别的东西：让用户重走一遍流程，
        // 而不是带着一句"内部错误"停在原地。
        let blob = URL_SAFE_NO_PAD
            .decode(pending.as_bytes())
            .map_err(|_| AppError::ValidationFailed("绑定已失效，请重新开始绑定".to_string()))?;
        let secret = totp::decrypt_secret(&blob, &self.encryption_key)?;

        // 必须用**用户 App 生成的码**来证明 App 侧配置成功。
        // 不做这一步的话，用户可以绑一把自己都不知道明文的密钥，
        // 然后在第一次需要输码时发现自己登不进去。
        if totp::verify_code(&secret, code)?.is_none() {
            return Err(AppError::ValidationFailed(
                "验证码不正确，请确认手机时间准确后重试".to_string(),
            ));
        }

        let encrypted = totp::encrypt_secret(&secret, &self.encryption_key)?;
        self.repo.enable(user_id, encrypted).await?;
        let recovery_codes = self.issue_recovery_codes(user_id).await?;
        // 落库成功后才消费待确认密钥：中途失败时用户还能用同一把密钥重试，
        // 而密钥已经进库时留着它只会让"重新 setup"拿不到旧密钥之外的东西。
        redis.take_string(&key).await?;
        Ok(EnableTwoFactorResponse { recovery_codes })
    }

    /// 关闭 2FA（需出示当前口令）
    pub async fn disable(
        &self,
        user_id: Uuid,
        user_repo: &crate::repository::user::UserRepository,
        req: &DisableTwoFactorRequest,
    ) -> Result<(), AppError> {
        // 先验口令再查状态：口令不对时不该顺带泄露"这个账号开没开 2FA"。
        let user = user_repo.find_by_id(user_id).await?;
        let ok = matches!(
            crate::utils::password::check_password(&req.password, &user.password_hash)
                .map_err(|e| AppError::InternalServerError(e.to_string()))?,
            crate::utils::password::PasswordCheck::Valid
                | crate::utils::password::PasswordCheck::ValidNeedsUpgrade(_)
        );
        if !ok {
            return Err(AppError::ValidationFailed("当前密码不正确".to_string()));
        }
        if !self.is_enabled(user_id).await? {
            return Err(AppError::ValidationFailed(NOT_ENABLED.to_string()));
        }
        self.repo.disable(user_id).await?;
        Ok(())
    }

    /// 重新生成恢复码（旧的一批立即作废）
    pub async fn regenerate_recovery_codes(
        &self,
        user_id: Uuid,
    ) -> Result<RecoveryCodesResponse, AppError> {
        if !self.is_enabled(user_id).await? {
            return Err(AppError::ValidationFailed(NOT_ENABLED.to_string()));
        }
        let codes = self.issue_recovery_codes(user_id).await?;
        Ok(RecoveryCodesResponse {
            recovery_codes: codes,
        })
    }

    /// 生成并持久化一批恢复码，返回明文
    async fn issue_recovery_codes(&self, user_id: Uuid) -> Result<Vec<String>, AppError> {
        let codes = totp::generate_recovery_codes(totp::recovery_code_count());
        let hashes: Vec<String> = codes.iter().map(|c| totp::hash_recovery_code(c)).collect();
        self.repo.replace_recovery_codes(user_id, &hashes).await?;
        Ok(codes)
    }

    // ── 登录时的二次校验 ────────────────────────────────────────

    /// 为一次已通过口令校验的登录签发挑战令牌
    pub async fn issue_challenge(
        &self,
        redis: &RedisClient,
        pending: &PendingLogin,
    ) -> Result<String, AppError> {
        let token = random_token()?;
        let payload = serde_json::to_string(pending)
            .map_err(|e| AppError::InternalServerError(format!("挑战令牌序列化失败: {e}")))?;
        redis
            .set_string(
                &format!("{CHALLENGE_PREFIX}{token}"),
                &payload,
                CHALLENGE_TTL_SECONDS,
            )
            .await?;
        Ok(token)
    }

    /// 消费挑战令牌并校验第二道因子，返回用户 ID
    ///
    /// 顺序刻意是**先消费令牌再校验码**：`take_string` 是原子的，
    /// 同一个挑战令牌第二次提交时已经查不到了。
    /// 反过来（先校验码再消费）会留下一个窗口——同一个码并发提交两次时，
    /// 两次都能通过校验，之后才在消费处发现令牌已失效，而两次请求都已走完全流程。
    pub async fn verify_challenge(
        &self,
        redis: &RedisClient,
        token: &str,
        code: &str,
    ) -> Result<PendingLogin, AppError> {
        let key = format!("{CHALLENGE_PREFIX}{token}");
        // 先**读**不删：验证码输错时用户应当能就地重试，
        // 不该每错一次就把口令也一起重输。
        let raw = redis
            .get_string(&key)
            .await?
            .ok_or_else(|| AppError::ValidationFailed(CHALLENGE_EXPIRED.to_string()))?;
        let pending: PendingLogin = serde_json::from_str(&raw)
            .map_err(|_| AppError::ValidationFailed(CHALLENGE_EXPIRED.to_string()))?;
        self.ensure_not_locked(redis, pending.user_id).await?;
        if !self
            .verify_second_factor(redis, pending.user_id, code)
            .await?
        {
            self.record_failure(redis, pending.user_id).await;
            return Err(AppError::ValidationFailed(
                "验证码不正确，请重试或使用恢复码".to_string(),
            ));
        }
        // 校验通过后才消费令牌，且**必须消费成功**：并发提交同一个挑战令牌时
        // 只有一次能拿到内容，另一次在这里发现令牌已消失，不会签发第二个令牌。
        redis
            .take_string(&key)
            .await?
            .ok_or_else(|| AppError::ValidationFailed(CHALLENGE_EXPIRED.to_string()))?;
        self.clear_failures(redis, pending.user_id).await;
        Ok(pending)
    }

    /// 二次验证失败次数是否已达阈值
    async fn ensure_not_locked(&self, redis: &RedisClient, user_id: Uuid) -> Result<(), AppError> {
        let max = self.setting_service.login_max_failures().await;
        if redis
            .login_failure_count(&Self::fail_scope(user_id))
            .await?
            >= max
        {
            return Err(AppError::TooManyRequests(
                "二次验证失败次数过多，请稍后再试".to_string(),
            ));
        }
        Ok(())
    }

    /// 记一次二次验证失败
    async fn record_failure(&self, redis: &RedisClient, user_id: Uuid) {
        let window = self.setting_service.login_failure_window_seconds().await;
        if let Err(e) = redis
            .record_login_failure(&Self::fail_scope(user_id), window)
            .await
        {
            tracing::warn!("二次验证失败计数写入失败 (user={user_id}): {e}");
        }
    }

    /// 清零二次验证失败计数
    async fn clear_failures(&self, redis: &RedisClient, user_id: Uuid) {
        if let Err(e) = redis.clear_login_failures(&Self::fail_scope(user_id)).await {
            tracing::warn!("二次验证失败计数清理失败 (user={user_id}): {e}");
        }
    }

    /// 校验第二道因子：动态验证码优先，其次恢复码
    ///
    /// 先试动态码再试恢复码，代价是恢复码路径多一次 Base32 解码，
    /// 换来的是用户不需要在"我这次该输哪种码"之间做选择。
    ///
    /// 两者都失败时返回 `false` 而**不**返回错误：调用方要据此记一次失败计数，
    /// 错误分支会绕过计数。
    pub async fn verify_second_factor(
        &self,
        redis: &RedisClient,
        user_id: Uuid,
        code: &str,
    ) -> Result<bool, AppError> {
        let state = self.repo.find_state(user_id).await?;
        if !state.is_enabled() {
            // 用户拿到挑战令牌之后关掉了 2FA：此时继续发令牌等于让一次
            // 已被撤销的第二因子形同虚设，要求重新登录。
            return Err(AppError::ValidationFailed(CHALLENGE_EXPIRED.to_string()));
        }
        let blob = state.secret_enc.expect("is_enabled 已保证密钥存在");
        let secret = totp::decrypt_secret(&blob, &self.encryption_key)?;

        if let Some(step) = totp::verify_code(&secret, code)? {
            // 防重放：同一个时间步只认一次（RFC 6238 §5.2）
            return redis.claim_totp_step(&user_id, step).await;
        }

        // 恢复码长度为 10，与 6 位动态码差得远，先挡掉绝大多数误输入再查库
        let trimmed = code.trim();
        if trimmed.len() >= 8 {
            let hash = totp::hash_recovery_code(trimmed);
            return self.repo.consume_recovery_code(user_id, &hash).await;
        }
        Ok(false)
    }

    /// 二次验证失败计数用的 scope
    ///
    /// 按**用户**而不是按 IP：6 位动态码只有 100 万种组合，
    /// 按 IP 计数挡不住"分散在大量 IP 上的分布式猜码"。
    pub fn fail_scope(user_id: Uuid) -> String {
        format!("{FAIL_SCOPE_PREFIX}{user_id}")
    }
}

/// 生成 32 字节随机挑战令牌（Base64url 编码）
fn random_token() -> Result<String, AppError> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf)
        .map_err(|e| AppError::InternalServerError(format!("随机源不可用: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_tokens_are_unique_and_url_safe() {
        let a = random_token().unwrap();
        let b = random_token().unwrap();
        assert_ne!(a, b, "两次随机不应撞车");
        // 32 字节 -> Base64url 无填充 43 字符；字符集里不应有 `+` `/` `=`，
        // 否则它进 JSON 或 URL 后还得额外转义一层
        assert_eq!(a.len(), 43);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn fail_scope_is_keyed_by_user() {
        let a = TwoFactorService::fail_scope(Uuid::nil());
        let b = TwoFactorService::fail_scope(Uuid::from_u128(1));
        assert_ne!(a, b);
        assert!(a.starts_with(FAIL_SCOPE_PREFIX));
    }
}
