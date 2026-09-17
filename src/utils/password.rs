//! 密码处理模块
//!
//! 使用 Argon2 算法（业界标准，OWASP 推荐）进行密码哈希与校验。
//! 自动生成盐值并编码到哈希结果中。
//!
//! 口令传输模型（v0.2 起）：客户端在 HTTPS 上直接提交明文口令，
//! 服务端只保存 `Argon2(明文)`。v0.1 保存的是 `Argon2(sha256(明文))`，
//! 为兼容存量账号，见 [`verify_password_with_legacy_upgrade`]。

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
    let parsed_hash = PasswordHash::new(hash).map_err(|e| anyhow!("无法解析密码哈希值: {}", e))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// 计算 SHA-256 十六进制串（仅用于兼容 v0.1 旧口令格式）
fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// 口令校验结果
#[derive(Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    /// 口令正确，且已是当前存储格式
    Valid,
    /// 口令正确，但存储的是 v0.1 旧格式；内部为应写回的新哈希
    ValidNeedsUpgrade(String),
    /// 口令错误
    Invalid,
}

/// 校验口令，并识别 v0.1 的旧口令格式
///
/// 这是一条临时迁移路径：存量账号全部升级后即可删除 `ValidNeedsUpgrade` 分支。
pub fn check_password(password: &str, stored_hash: &str) -> Result<PasswordCheck> {
    if verify_password(password, stored_hash)? {
        return Ok(PasswordCheck::Valid);
    }

    // 直接校验失败时，再按 v0.1 的 sha256 预哈希格式校验一次
    let legacy = sha256_hex(password);
    if verify_password(&legacy, stored_hash)? {
        return Ok(PasswordCheck::ValidNeedsUpgrade(hash_password(password)?));
    }
    Ok(PasswordCheck::Invalid)
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
    fn test_current_format_password_needs_no_upgrade() {
        let hash = hash_password("admin123").unwrap();
        assert_eq!(
            check_password("admin123", &hash).unwrap(),
            PasswordCheck::Valid
        );
        assert_eq!(
            check_password("wrong", &hash).unwrap(),
            PasswordCheck::Invalid
        );
    }

    #[test]
    fn test_legacy_sha256_hash_is_upgraded_once() {
        // 模拟 v0.1 存量数据：Argon2(sha256(明文))
        let legacy_hash = hash_password(&sha256_hex("admin123")).unwrap();

        let upgraded = match check_password("admin123", &legacy_hash).unwrap() {
            PasswordCheck::ValidNeedsUpgrade(new_hash) => new_hash,
            other => panic!("旧格式口令应要求升级，实际: {other:?}"),
        };

        // 新哈希必须是 Argon2(明文)，升级后不再需要回退
        assert_eq!(
            check_password("admin123", &upgraded).unwrap(),
            PasswordCheck::Valid
        );
        assert_eq!(
            check_password("wrong", &legacy_hash).unwrap(),
            PasswordCheck::Invalid
        );
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
