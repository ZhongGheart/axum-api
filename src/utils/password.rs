//! 密码处理模块
//!
//! 使用 Argon2 算法（业界标准，OWASP 推荐）进行密码哈希与校验。
//! 自动生成盐值并编码到哈希结果中。

use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// 对明文密码进行 Argon2 哈希
///
/// # Arguments
///
/// * `password` - 明文密码
///
/// # Returns
///
/// 返回 Argon2 格式的哈希字符串（包含盐值等元信息）。
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("密码哈希计算失败: {}", e))?;
    Ok(hash.to_string())
}

/// 验证密码与哈希值是否匹配
///
/// # Arguments
///
/// * `password` - 明文密码
/// * `hash` - Argon2 哈希字符串
///
/// # Returns
///
/// 匹配返回 `true`，否则返回 `false`。
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| anyhow!("无法解析密码哈希值: {}", e))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "my_secure_password_123";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong_password", &hash).unwrap());
    }

    #[test]
    fn test_different_hashes() {
        let password = "same_password";
        let hash1 = hash_password(password).unwrap();
        let hash2 = hash_password(password).unwrap();
        // 每次生成的哈希应不同（因为盐值不同）
        assert_ne!(hash1, hash2);
        // 但都能通过验证
        assert!(verify_password(password, &hash1).unwrap());
        assert!(verify_password(password, &hash2).unwrap());
    }
}
