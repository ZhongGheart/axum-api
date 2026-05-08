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
    /// 用户角色（admin / user）
    pub role: String,
    /// 签发时间（Unix 时间戳）
    pub iat: usize,
    /// 过期时间（Unix 时间戳）
    pub exp: usize,
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
    /// * `role` - 用户角色
    /// * `expiration_seconds` - 过期时间（秒）
    ///
    /// # Returns
    ///
    /// 返回签发的 JWT 字符串。
    pub fn sign(
        &self,
        user_id: Uuid,
        role: &str,
        expiration_seconds: u64,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: user_id,
            role: role.to_string(),
            iat: now,
            exp: now + expiration_seconds as usize,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
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
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
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
        let token = jwt.sign(user_id, "user", 3600).unwrap();
        let claims = jwt.verify(&token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.role, "user");
    }

    #[test]
    fn test_invalid_token() {
        let jwt = JwtUtil::new("test_secret_key");
        let result = jwt.verify("invalid_token_here");
        assert!(result.is_err());
    }
}
