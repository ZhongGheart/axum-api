//! 两步验证（TOTP）核心能力
//!
//! 三件事，各自的边界写在这里而不是散在 service 里：
//!
//! 1. **密钥保管**：TOTP 密钥必须可逆（要拿它算验证码），因此与口令不同，
//!    不能只存 Argon2 摘要。这里用 AES-256-GCM 加密后落库，密钥来自
//!    `TOTP_ENCRYPTION_KEY`，与数据库分离。
//! 2. **验证码校验**：允许 ±1 个时间窗（各 30 秒）的漂移。
//! 3. **恢复码**：一次性后门，只存 SHA-256 摘要。
//!
//! 刻意不自己实现 HMAC/TOTP 运算——`totp-rs` 是经过验证的实现，
//! 手写一遍只会得到一个"看起来能跑"的 RFC 6238 近似品。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base32::Alphabet;
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Builder, Totp};

use crate::error::AppError;

/// TOTP 参数：6 位数字、30 秒步长、SHA-1（Google Authenticator 等主流 App 的默认）
const TOTP_DIGITS: u8 = 6;
const TOTP_PERIOD: u64 = 30;

/// 允许的时间窗漂移：±1（共 3 个候选码）
///
/// 0 会在手机时钟差几百毫秒时随机失败；±2 会让一个码的有效期长达 150 秒，
/// 明显削弱第二因子的意义。±1 是通行取值。
const TOTP_SKEW: u16 = 1;

/// 恢复码字符集：去掉了 `0/O`、`1/I/L` 等手抄易混字符
const RECOVERY_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const RECOVERY_CODE_LEN: usize = 10;
const RECOVERY_CODE_COUNT: usize = 8;

/// 生成新的 TOTP 密钥（Base32 编码的 20 字节随机数）
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 20];
    // 显式取操作系统 CSPRNG，让"这是安全随机源"在代码里看得见，
    // 而不是依赖调用方记得该用哪个 RNG。
    fill_random(&mut bytes).expect("操作系统随机源不可用");
    base32::encode(Alphabet::Rfc4648 { padding: false }, &bytes)
}

/// 从操作系统 CSPRNG 取随机字节
fn fill_random(buf: &mut [u8]) -> Result<(), AppError> {
    getrandom::fill(buf).map_err(|e| AppError::InternalServerError(format!("随机源不可用: {e}")))
}

/// 把 Base32 密钥还原成 `Totp` 实例
fn totp_from_secret(secret: &str) -> Result<Totp, AppError> {
    let bytes = decode_secret(secret)?;
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(TOTP_DIGITS)
        .with_skew(TOTP_SKEW)
        .with_step_duration(TOTP_PERIOD)
        .with_secret(bytes)
        .build()
        .map_err(|e| AppError::InternalServerError(format!("TOTP 初始化失败: {e}")))
}

/// 解 Base32 密钥
fn decode_secret(secret: &str) -> Result<Vec<u8>, AppError> {
    base32::decode(Alphabet::Rfc4648 { padding: false }, secret.trim())
        .ok_or_else(|| AppError::InternalServerError("TOTP 密钥不是合法 Base32".to_string()))
}

/// 生成 `otpauth://` 供验证器 App 扫码的 URI
pub fn provisioning_uri(secret: &str, username: &str, issuer: &str) -> String {
    // 扫码 URI 必须带账号名与签发方，两者都由 Builder 提供——
    // 少了账号名，验证器 App 里这条条目就只是个无主密钥。
    match decode_secret(secret) {
        Ok(bytes) => Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_digits(TOTP_DIGITS)
            .with_step_duration(TOTP_PERIOD)
            .with_secret(bytes)
            .with_account_name(username)
            .with_issuer(Some(issuer))
            .build()
            .ok()
            .and_then(|t| t.to_url().ok()),
        // 密钥不是合法 Base32 才走到这里，属于调用方 bug（密钥由本模块生成）。
        // 返回空串而不是 panic：绑定流程降级为"手动输入密钥"，不至于让接口 500。
        Err(_) => None,
    }
    .unwrap_or_default()
}

/// 校验用户提交的验证码
///
/// 返回命中的**时间步**（step）而非简单的 bool，调用方据此做防重放：
/// RFC 6238 §5.2 明确要求一个码只被接受一次，而 TOTP 本身不保证这点——
/// 同一个 6 位码在它 90 秒的有效期（step ± 1）内可以被无限次用于登录。
/// 把 step 返回给上层，上层用"已用过的最大 step"挡住重放。
pub fn verify_code(secret: &str, code: &str) -> Result<Option<u64>, AppError> {
    let code = code.trim().replace(' ', "");
    if code.len() != TOTP_DIGITS as usize || !code.chars().all(|c| c.is_ascii_digit()) {
        // 格式错误不算"校验失败"这个业务结论，但它同样不该进入验证码比对：
        // 用户可能只是多按了一次回车，不该因此被计入失败次数。
        return Ok(None);
    }
    let totp = totp_from_secret(secret)?;
    Ok(totp.check_current(&code))
}

/// 取某个密钥在给定时刻的验证码（集成测试与单测用来造出合法码）
pub fn code_at(secret: &str, unix_seconds: u64) -> Result<String, AppError> {
    let totp = totp_from_secret(secret)?;
    Ok(totp.generate(unix_seconds).to_string())
}

// ── 密钥加密 ─────────────────────────────────────────────────────

/// 把配置里的密钥规整成 AES-256 需要的 `[u8; 32]`
fn key_bytes(secret: &str) -> Result<[u8; 32], AppError> {
    // 正好 32 字节时按字面值用（运维用 `openssl rand -hex 16` 之类生成的串）。
    // 长度不足则退化到 SHA-256 派生：任意长度的配置值都能工作，
    // 且派生是确定性的——同一份配置永远解出同一把密钥。
    // 这里**不**因为长度不符就报错，否则一次发版就能让全部已绑定用户的
    // 密钥变成永久不可解（他们没有任何自助恢复路径）。
    if secret.len() == 32 {
        let mut out = [0u8; 32];
        out.copy_from_slice(secret.as_bytes());
        return Ok(out);
    }
    let digest = Sha256::digest(secret.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

/// 加密 TOTP 密钥，输出 `nonce || ciphertext || tag`
pub fn encrypt_secret(secret: &str, config_key: &str) -> Result<Vec<u8>, AppError> {
    let key = key_bytes(config_key)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let mut nonce_bytes = [0u8; 12];
    // GCM 下 nonce 绝不能重复：同一把密钥下重复使用会彻底破坏机密性。
    fill_random(&mut nonce_bytes)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: secret.as_bytes(),
                aad: b"axum-api/totp-secret/v1",
            },
        )
        .map_err(|_| AppError::InternalServerError("TOTP 密钥加密失败".to_string()))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密 TOTP 密钥
pub fn decrypt_secret(blob: &[u8], config_key: &str) -> Result<String, AppError> {
    if blob.len() < 12 + 16 {
        // 最短合法密文 = 12 字节 nonce + 16 字节 tag
        return Err(AppError::InternalServerError(
            "TOTP 密文长度非法".to_string(),
        ));
    }
    let key = key_bytes(config_key)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload {
                msg: ciphertext,
                aad: b"axum-api/totp-secret/v1",
            },
        )
        // 解密失败只有两种原因：配置密钥不对，或密文被篡改。
        // 两者都不该对外暴露细节——统一按"解不开"表述。
        .map_err(|_| {
            AppError::InternalServerError(
                "TOTP 密钥解密失败（配置密钥不匹配或数据已损坏）".to_string(),
            )
        })?;
    String::from_utf8(plaintext)
        .map_err(|_| AppError::InternalServerError("TOTP 密钥明文非法".to_string()))
}

// ── 恢复码 ───────────────────────────────────────────────────────

/// 生成一批恢复码明文（**仅此一次**返回给用户，之后不可再取）
pub fn generate_recovery_codes(count: usize) -> Vec<String> {
    let mut buf = vec![0u8; count * RECOVERY_CODE_LEN];
    fill_random(&mut buf).expect("操作系统随机源不可用");
    buf.chunks(RECOVERY_CODE_LEN)
        .map(|chunk| {
            chunk
                .iter()
                .map(|b| RECOVERY_ALPHABET[(*b as usize) % RECOVERY_ALPHABET.len()] as char)
                .collect()
        })
        .collect()
}

/// 单批恢复码数量
pub fn recovery_code_count() -> usize {
    RECOVERY_CODE_COUNT
}

/// 恢复码摘要
///
/// 归一化（去分隔符、转大写）后再摘要：用户会把恢复码抄下来、
/// 可能写成 `ABCD-EFGH IJ`，不归一就会因为格式差异而认不出来。
pub fn hash_recovery_code(code: &str) -> String {
    let normalized = code.trim().to_uppercase().replace(['-', ' '], "");
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn secret_roundtrips_through_aes_gcm() {
        let secret = generate_secret();
        let blob = encrypt_secret(&secret, KEY).unwrap();
        assert_eq!(decrypt_secret(&blob, KEY).unwrap(), secret);
    }

    #[test]
    fn ciphertext_is_not_the_plaintext_secret() {
        let secret = generate_secret();
        let blob = encrypt_secret(&secret, KEY).unwrap();
        assert_ne!(blob, secret.as_bytes());
        // 每次加密 nonce 不同，因此同一密钥两次加密的密文不同
        let blob2 = encrypt_secret(&secret, KEY).unwrap();
        assert_ne!(blob, blob2);
    }

    #[test]
    fn wrong_config_key_cannot_decrypt() {
        let blob = encrypt_secret(&generate_secret(), KEY).unwrap();
        let err = decrypt_secret(&blob, "ffffffffffffffffffffffffffffffff").unwrap_err();
        assert!(err.to_string().contains("解密失败"));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let secret = generate_secret();
        let mut blob = encrypt_secret(&secret, KEY).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(decrypt_secret(&blob, KEY).is_err());
    }

    #[test]
    fn non_32_byte_config_key_is_accepted_via_derivation() {
        let secret = generate_secret();
        let blob = encrypt_secret(&secret, "short").unwrap();
        assert_eq!(decrypt_secret(&blob, "short").unwrap(), secret);
    }

    #[test]
    fn generated_secret_is_valid_base32_of_expected_len() {
        let secret = generate_secret();
        assert!(secret
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        // 20 字节 -> Base32(无填充) 32 字符
        assert_eq!(secret.len(), 32);
    }

    #[test]
    fn provisioning_uri_carries_issuer_and_account() {
        let secret = generate_secret();
        let uri = provisioning_uri(&secret, "alice", "Axum Api");
        assert!(uri.starts_with("otpauth://totp/"), "got: {uri}");
        assert!(uri.contains("secret="));
        assert!(uri.contains("issuer=Axum%20Api"));
        assert!(uri.contains("alice"));
    }

    #[test]
    fn verification_accepts_current_code_and_returns_its_step() {
        let secret = generate_secret();
        let now = chrono::Utc::now().timestamp() as u64;
        let code = code_at(&secret, now).unwrap();
        let step = verify_code(&secret, &code).unwrap().expect("当前码应通过");
        // step 必须落在"当前时间窗 ± 1"内，这正是 TOTP_SKEW 的含义
        let current = now / TOTP_PERIOD;
        let diff = current.abs_diff(step);
        assert!(diff <= TOTP_SKEW as u64, "step 偏离当前窗口 {diff} 步");
    }

    #[test]
    fn verification_accepts_neighbouring_window_for_clock_drift() {
        let secret = generate_secret();
        let now = chrono::Utc::now().timestamp() as u64;
        // 上一个时间窗的码：手机时钟慢几十秒时用户看到的就是它
        let prev = code_at(&secret, now - TOTP_PERIOD).unwrap();
        assert!(verify_code(&secret, &prev).unwrap().is_some());
        // 超出 ±1 窗口则必须被拒
        let stale = code_at(&secret, now - TOTP_PERIOD * 5).unwrap();
        assert!(verify_code(&secret, &stale).unwrap().is_none());
    }

    #[test]
    fn verification_rejects_malformed_codes_without_comparing() {
        let secret = generate_secret();
        // 格式非法的输入直接判否，且不进入比对
        assert!(verify_code(&secret, "abcdef").unwrap().is_none());
        assert!(verify_code(&secret, "12345").unwrap().is_none());
        assert!(verify_code(&secret, "").unwrap().is_none());
        assert!(verify_code(&secret, "1234567").unwrap().is_none());
        // 非本密钥的码
        let other = code_at(&generate_secret(), chrono::Utc::now().timestamp() as u64).unwrap();
        assert!(verify_code(&secret, &other).unwrap().is_none());
    }

    #[test]
    fn verification_tolerates_surrounding_spaces() {
        let secret = generate_secret();
        let code = code_at(&secret, chrono::Utc::now().timestamp() as u64).unwrap();
        assert!(verify_code(&secret, &format!(" {} ", code))
            .unwrap()
            .is_some());
    }

    #[test]
    fn recovery_codes_are_distinct_and_normalizable() {
        let codes = generate_recovery_codes(recovery_code_count());
        assert_eq!(codes.len(), 8);
        assert_eq!(
            codes.iter().collect::<std::collections::HashSet<_>>().len(),
            8
        );
        for c in &codes {
            assert_eq!(c.len(), RECOVERY_CODE_LEN);
            assert!(c.chars().all(|ch| RECOVERY_ALPHABET.contains(&(ch as u8))));
        }
        // 大小写与分隔符不影响摘要匹配
        assert_eq!(
            hash_recovery_code("abcd-efgh ij"),
            hash_recovery_code("ABCDEFGHIJ")
        );
    }

    #[test]
    fn recovery_hashes_are_sha256_hex() {
        let h = hash_recovery_code("ABCD234567");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
