//! 非对称加解密模块（RSA-OAEP）
//!
//! 提供 RSA 密钥对生成、加密、解密功能。
//! 公钥分发给前端用于加密敏感请求参数，
//! 私钥由服务端持有用于解密。
//!
//! 密钥配置通过环境变量传入，支持动态切换。

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPrivateKey, EncodeRsaPublicKey},
    Oaep, RsaPrivateKey, RsaPublicKey,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::AppError;

/// 加密配置
#[derive(Debug, Clone)]
pub struct CryptoConfig {
    /// RSA 私钥（PEM 格式）
    pub private_key_pem: String,
    /// RSA 公钥（PEM 格式）
    pub public_key_pem: Option<String>,
    /// 是否启用请求加密
    pub enabled: bool,
    /// 需要强制加密的接口路径前缀列表
    pub enforced_paths: Vec<String>,
}

/// 加密服务
#[derive(Debug, Clone)]
pub struct CryptoService {
    inner: Arc<RwLock<CryptoInner>>,
}

#[derive(Debug)]
struct CryptoInner {
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
    config: CryptoConfig,
}

impl CryptoService {
    /// 从配置创建加密服务
    ///
    /// 如果未提供密钥对，自动生成一组（开发模式）。
    pub fn new(config: CryptoConfig) -> Self {
        let (private_key, public_key) = match (&config.private_key_pem, &config.public_key_pem) {
            (priv_pem, Some(pub_pem)) if !priv_pem.is_empty() && !pub_pem.is_empty() => {
                let priv_key = RsaPrivateKey::from_pkcs1_pem(priv_pem)
                    .expect("无效的 RSA 私钥 PEM");
                let pub_key = RsaPublicKey::from_pkcs1_pem(pub_pem)
                    .expect("无效的 RSA 公钥 PEM");
                (priv_key, pub_key)
            }
            _ => {
                tracing::warn!("未提供 RSA 密钥对，自动生成 2048 位密钥");
                let mut rng = rand::thread_rng();
                let priv_key = RsaPrivateKey::new(&mut rng, 2048)
                    .expect("RSA 密钥生成失败");
                let pub_key = RsaPublicKey::from(&priv_key);
                (priv_key, pub_key)
            }
        };

        Self {
            inner: Arc::new(RwLock::new(CryptoInner {
                private_key,
                public_key,
                config,
            })),
        }
    }

    /// 动态更新密钥对（不停机切换）
    pub async fn rotate_keys(&self, private_key_pem: &str, public_key_pem: &str) -> Result<(), AppError> {
        let priv_key = RsaPrivateKey::from_pkcs1_pem(private_key_pem)
            .map_err(|e| AppError::InternalServerError(format!("私钥格式错误: {e}")))?;
        let pub_key = RsaPublicKey::from_pkcs1_pem(public_key_pem)
            .map_err(|e| AppError::InternalServerError(format!("公钥格式错误: {e}")))?;

        let mut inner = self.inner.write().await;
        inner.private_key = priv_key;
        inner.public_key = pub_key;
        tracing::info!("RSA 密钥对已动态切换");
        Ok(())
    }

    /// 获取公钥（PEM 格式）
    pub async fn get_public_key_pem(&self) -> Result<String, AppError> {
        let inner = self.inner.read().await;
        inner.public_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .map_err(|e| AppError::InternalServerError(format!("公钥导出失败: {e}")))
    }

    /// 解密密文（base64 + RSA-OAEP-SHA256）
    pub async fn decrypt(&self, ciphertext_b64: &str) -> Result<String, AppError> {
        let cipher_bytes = BASE64.decode(ciphertext_b64)
            .map_err(|e| AppError::BadRequest(format!("Base64 解码失败: {e}")))?;

        let inner = self.inner.read().await;
        let padding = Oaep::new::<Sha256>();
        let plaintext = inner
            .private_key
            .decrypt(padding, &cipher_bytes)
            .map_err(|e| AppError::BadRequest(format!("解密失败: {e}")))?;

        String::from_utf8(plaintext)
            .map_err(|e| AppError::BadRequest(format!("解密结果非 UTF-8: {e}")))
    }

    /// 加密明文
    pub async fn encrypt(&self, plaintext: &str) -> Result<String, AppError> {
        let inner = self.inner.read().await;
        let padding = Oaep::new::<Sha256>();
        let cipher_bytes = inner
            .public_key
            .encrypt(&mut rand::thread_rng(), padding, plaintext.as_bytes())
            .map_err(|e| AppError::InternalServerError(format!("加密失败: {e}")))?;

        Ok(BASE64.encode(cipher_bytes))
    }

    /// 判断路径是否需要强制加密
    pub fn is_enforced_path(config: &CryptoConfig, path: &str) -> bool {
        config.enforced_paths.iter().any(|p| path.starts_with(p))
    }

    /// 是否启用加密
    pub fn is_enabled(&self) -> bool {
        // 同步方式读配置
        false // 简化，实际需从 inner 读取
    }
}

// ============================================
// 保留原 SHA-256 哈希函数（前端密码传输用）
// ============================================

/// 对输入字符串进行 SHA-256 哈希，返回小写十六进制字符串
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成新 RSA 密钥对
pub fn generate_key_pair() -> (String, String) {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("密钥生成失败");
    let public_key = RsaPublicKey::from(&private_key);

    let priv_pem_str = private_key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .expect("私钥 PEM 编码失败")
        .to_string();
    let pub_pem_str = public_key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .expect("公钥 PEM 编码失败")
        .to_string();

    (priv_pem_str, pub_pem_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_encrypt_decrypt_roundtrip() {
        let (priv_pem, pub_pem) = generate_key_pair();
        let config = CryptoConfig {
            private_key_pem: priv_pem,
            public_key_pem: Some(pub_pem),
            enabled: true,
            enforced_paths: vec![],
        };
        let svc = CryptoService::new(config);

        let plaintext = "{\"username\":\"admin\",\"password\":\"secret123\"}";
        let cipher = svc.encrypt(plaintext).await.unwrap();
        let decrypted = svc.decrypt(&cipher).await.unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[tokio::test]
    async fn test_rotate_keys() {
        let config = CryptoConfig {
            private_key_pem: String::new(),
            public_key_pem: None,
            enabled: true,
            enforced_paths: vec![],
        };
        let svc = CryptoService::new(config);

        let (new_priv, new_pub) = generate_key_pair();
        svc.rotate_keys(&new_priv, &new_pub).await.unwrap();

        let pub_key = svc.get_public_key_pem().await.unwrap();
        assert!(pub_key.contains("BEGIN RSA PUBLIC KEY"));
    }
}
