//! JWT 令牌管理模块
//!
//! 提供 JWT 的签发（sign）与验证（verify）功能。
//! 使用 HS256 算法，将用户 ID 和角色编码到令牌中。

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JWT 载荷（Claims）
///
/// 包含声明在令牌中的用户信息。
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// 用户 ID
    pub sub: Uuid,
    /// 用户主要角色（向后兼容，保持与旧令牌的一致性）
    pub role: String,
    /// 用户拥有的所有角色标识列表（RBAC 权限判断依据）
    pub roles: Vec<String>,
    /// 令牌唯一标识（用于单令牌注销）
    pub jti: String,
    /// 用户名（仅用于审计日志展示，不作为鉴权依据）
    pub username: String,
    /// 签发时间（Unix 时间戳，JWT 标准要求 u64）
    pub iat: u64,
    /// 签发时间（Unix **毫秒**时间戳）
    ///
    /// 只用于会话吊销比对，不能拿它替代 `iat`：`exp` 的校验由
    /// jsonwebtoken 按标准 `iat`/`exp`（秒）完成。
    ///
    /// 之所以要多一个claim，是因为 JWT 标准的 `iat` 只有**秒**级精度，
    /// 而吊销时间点若也只存到秒，就分不清"令牌在吊销之前签发"与
    /// "令牌在吊销之后签发"——两者可能落在同一秒里。此时任何秒级方案
    /// 都必须二选一：要么放过旧令牌（漏吊销），要么误伤新登录的令牌。
    /// 实测正是如此：登录后立刻改角色，旧令牌仍可用。
    ///
    /// `serde(default)` 让升级前签发的旧令牌仍能解析（此时为 0），
    /// 且 0 一定小于任何吊销时间点 —— 方向是**失效**而非放行，
    /// 即升级后首次吊销会把存量令牌一并作废，这是安全的一侧。
    #[serde(default)]
    pub iat_ms: u64,
    /// 令牌为"受限令牌"：用户必须先改密，令牌只能调用改密/登出/`/me`
    ///
    /// v0.11.0 新增。**为什么不改成"登录即拒绝、逼用户先改密"**：
    /// 那会让用户拿不到令牌，也就无法调用改密接口——
    /// 要么死锁，要么就得再开一条无认证的改密通道（那才是真正的洞）。
    /// 签发受限令牌既不需要第二次输密码，也不丢弃用户已有会话。
    ///
    /// `serde(default)` 让升级前的存量令牌解析为 `false`（即不受限），
    /// 方向与 `iat_ms` 一致地落在**不误伤**的一侧。
    #[serde(default)]
    pub pwd_stale: bool,
    /// 过期时间（Unix 时间戳，JWT 标准要求 u64）
    pub exp: u64,
}

/// JWT 工具结构体
#[derive(Debug, Clone)]
pub struct JwtUtil {
    secret: String,
}

impl JwtUtil {
    /// 创建新的 JWT 工具实例
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// 签发 JWT 令牌
    ///
    /// # Arguments
    ///
    /// * `user_id` - 用户 UUID
    /// * `username` - 用户名
    /// * `role` - 用户主要角色
    /// * `roles` - 用户拥有的所有角色标识列表
    /// * `expiration_seconds` - 过期时间（秒）
    ///
    /// # Returns
    ///
    /// 返回签发的 JWT 字符串。
    pub fn sign(
        &self,
        user_id: Uuid,
        username: &str,
        role: &str,
        roles: &[String],
        expiration_seconds: u64,
        pwd_stale: bool,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        self.sign_with_jti(
            user_id,
            username,
            role,
            roles,
            expiration_seconds,
            pwd_stale,
        )
        .map(|(token, _)| token)
    }

    /// 签发令牌并**同时返回 jti**
    ///
    /// v0.20.0 新增。登录后要把这个会话登记进 Redis，而登记的键里必须有 jti。
    /// 早先只能拿到令牌字符串——jti 被封在 JWT 载荷里，
    /// 调用方若选择"自己解一次 payload 取 jti"，等于多一份解析路径，
    /// 且解出来的值未经校验就进了 Redis 的键。
    ///
    /// 保留 [`Self::sign`] 作为薄封装，让既有调用点与测试不必改。
    #[allow(clippy::type_complexity)]
    pub fn sign_with_jti(
        &self,
        user_id: Uuid,
        username: &str,
        role: &str,
        roles: &[String],
        expiration_seconds: u64,
        pwd_stale: bool,
    ) -> Result<(String, String), jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now().timestamp() as u64;
        let jti = Uuid::new_v4().to_string();
        let claims = Claims {
            sub: user_id,
            role: role.to_string(),
            roles: roles.to_vec(),
            jti: jti.clone(),
            username: username.to_string(),
            iat: now,
            iat_ms: chrono::Utc::now().timestamp_millis() as u64,
            pwd_stale,
            exp: now + expiration_seconds,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )?;
        Ok((token, jti))
    }

    /// 验证 JWT 令牌并返回载荷
    ///
    /// # Arguments
    ///
    /// * `token` - JWT 令牌字符串
    ///
    /// # Returns
    ///
    /// 验证通过返回 `Claims`，失败返回错误。
    pub fn verify(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let mut validation = Validation::default();
        // 显式设置为 HS256（与签发算法一致）
        validation.algorithms = vec![jsonwebtoken::Algorithm::HS256];
        // 不设置 leeway（默认60s），严格过期校验
        validation.leeway = 0;

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &validation,
        )?;
        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_sign_and_verify() {
        let jwt = JwtUtil::new("test_secret_key");
        let user_id = Uuid::new_v4();
        let roles = vec!["user".to_string(), "admin".to_string()];
        let token = jwt
            .sign(user_id, "alice", "admin", &roles, 3600, false)
            .unwrap();
        let claims = jwt.verify(&token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.role, "admin");
        assert!(claims.roles.contains(&"user".to_string()));
        assert!(claims.roles.contains(&"admin".to_string()));
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn test_each_token_has_unique_jti() {
        let jwt = JwtUtil::new("test_secret_key");
        let user_id = Uuid::new_v4();
        let a = jwt
            .sign(user_id, "alice", "user", &[], 3600, false)
            .unwrap();
        let b = jwt
            .sign(user_id, "alice", "user", &[], 3600, false)
            .unwrap();
        assert_ne!(
            jwt.verify(&a).unwrap().jti,
            jwt.verify(&b).unwrap().jti,
            "同一用户的不同令牌必须具有不同 jti，才能单令牌注销"
        );
    }

    #[test]
    fn test_invalid_token() {
        let jwt = JwtUtil::new("test_secret_key");
        let result = jwt.verify("invalid_token_here");
        assert!(result.is_err());
    }
}
