//! S3 兼容对象存储后端
//!
//! 用 [opendal] 而不是直接上 `aws-sdk-s3`：后者要自己拼 SigV4、
//! 自己处理分片与重试，而头像这种"小文件、一次性、无并发"的对象
//! 用不上那些能力。opendal 把 S3 / MinIO / OSS / COS 都收敛到同一套 API，
//! 换厂商只改环境变量。
//!
//! ── 为什么 `public_base_url` 是独立配置而不是从 endpoint 推 ──
//!
//! 私有 bucket 的对象 URL **不能直接给浏览器**：没有签名就是 403。
//! 生产上一般在前面挂 CDN，此时 `avatar_url` 必须是 CDN 域名。
//! 而把 endpoint 拼出来的地址直发给浏览器，只在 bucket 公开读时成立——
//! 那恰好是最不该用的配置。所以这里要求显式给出，不猜。

use async_trait::async_trait;
use opendal::services::S3;
use opendal::Operator;

use super::{avatar_key, content_type_for, LoadedObject, SavedObject, Storage};
use crate::config::{S3Config, StorageConfig};
use crate::error::AppError;

pub struct S3Storage {
    op: Operator,
    /// bucket 内的 key 前缀，形如 `avatars` 或 `axum/prod`
    prefix: String,
    /// 对外访问基地址
    base: String,
}

impl S3Storage {
    pub fn new(config: &StorageConfig) -> Result<Self, AppError> {
        let s3: S3Config = config
            .s3
            .clone()
            .ok_or_else(|| AppError::InternalServerError("S3 后端缺少配置".into()))?;

        // 0.59 的 `S3` 是 builder，链式方法而非字段赋值
        let mut builder = S3::default()
            .root("/")
            // bucket 必填：不给会得到 `ConfigInvalid: The bucket is misconfigured`
            .bucket(&s3.bucket)
            .region(&s3.region)
            .access_key_id(&s3.access_key_id)
            .secret_access_key(&s3.secret_access_key)
            // 不去读 ~/.aws/config：容器里通常没有，而一旦有了一份过期的
            // profile 就会静默改掉 endpoint，排查起来极难
            .disable_config_load();
        if let Some(endpoint) = s3.endpoint.as_deref() {
            // opendal 0.59 默认就是 path-style（`endpoint/bucket/key`），
            // 这正是 MinIO / OSS / COS 认的形式；虚拟主机式要显式 enable。
            builder = builder.endpoint(endpoint);
        }

        // opendal 要求显式安装默认 HTTP transport，
        // 否则 operator 一用就报 "default HTTP transport is not installed"
        opendal::install_default();
        // 0.59 的 `Operator::new` 直接返回 Operator，没有 `.finish()`
        let op = Operator::new(builder)
            .map_err(|e| AppError::InternalServerError(format!("初始化 S3 存储失败: {e}")))?;

        let bucket = &s3.bucket;
        let region = &s3.region;
        let base = match s3.public_base_url.as_deref() {
            Some(b) => b.to_string(),
            None => match s3.endpoint.as_deref() {
                Some(e) => format!("{e}/{bucket}"),
                None => format!("https://{bucket}.s3.{region}.amazonaws.com"),
            },
        };

        Ok(Self {
            op,
            prefix: s3.key_prefix,
            base,
        })
    }

    /// 把逻辑 key 拼上 bucket 内前缀
    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.prefix, key)
        }
    }
}

/// 手写而非 derive：[`Operator`] 没实现 `Debug`。
///
/// 只打后端自己的两个字段，不打凭据——`AppState` 是 derive 了 `Debug` 的，
/// 一旦有人 `dbg!(state)` 或把 state 塞进 panic 日志，
/// `secret_access_key` 就会跟着进日志文件。
impl std::fmt::Debug for S3Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Storage")
            .field("prefix", &self.prefix)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Storage for S3Storage {
    async fn put(&self, mime: &str, bytes: &[u8]) -> Result<SavedObject, AppError> {
        if bytes.is_empty() {
            return Err(AppError::BadRequest("图片内容为空".into()));
        }
        let key = avatar_key(mime)?;
        self.op
            .write(&self.full_key(&key), bytes.to_vec())
            .await
            .map_err(|e| AppError::InternalServerError(format!("写入对象存储失败: {e}")))?;
        Ok(SavedObject {
            url: self.public_url(&key),
            key,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        match self.op.delete(&self.full_key(key)).await {
            Ok(()) => Ok(()),
            // 删一个已经不在的对象必须成功：替换头像时旧文件可能已被并发请求删掉，
            // 把"已经没了"报成错误会让一次成功的上传变成 500
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::InternalServerError(format!(
                "删除对象存储对象失败: {e}"
            ))),
        }
    }

    async fn get(&self, key: &str) -> Result<Option<LoadedObject>, AppError> {
        match self.op.read(&self.full_key(key)).await {
            Ok(buf) => Ok(Some(LoadedObject {
                bytes: buf.to_bytes().to_vec(),
                content_type: content_type_for(key),
            })),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::InternalServerError(format!(
                "读取对象存储失败: {e}"
            ))),
        }
    }

    fn public_url(&self, key: &str) -> String {
        format!("{}/{}", self.base, self.full_key(key))
    }

    fn key_of_url(&self, url: &str) -> Option<String> {
        super::key_of_url_under(url, &self.base, &self.prefix)
    }

    fn backend_name(&self) -> &'static str {
        "s3"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(prefix: &str) -> StorageConfig {
        StorageConfig {
            backend: crate::config::StorageBackend::S3,
            dir: "./uploads".to_string(),
            max_file_size: 1024,
            allowed_mime_types: vec!["image/png".to_string()],
            s3: Some(S3Config {
                endpoint: Some("http://127.0.0.1:59000".to_string()),
                bucket: "axum-test".to_string(),
                region: "us-east-1".to_string(),
                access_key_id: "testkey".to_string(),
                secret_access_key: "testsecret".to_string(),
                public_base_url: Some("https://cdn.example.com".to_string()),
                key_prefix: prefix.to_string(),
            }),
        }
    }

    fn store(prefix: &str) -> S3Storage {
        S3Storage::new(&cfg(prefix)).unwrap()
    }

    /// 不需要网络：只验 URL 拼装
    #[test]
    fn public_urls_are_cdn_addresses_under_the_configured_base() {
        let s = store("avatars");
        assert_eq!(
            s.public_url("avatars/a.png"),
            "https://cdn.example.com/avatars/avatars/a.png"
        );
    }

    /// 没给 `S3_PUBLIC_BASE_URL` 时退回 endpoint+bucket，
    /// 但那只在 bucket 公开读时成立（见模块文档）
    #[test]
    fn the_base_falls_back_to_endpoint_and_bucket() {
        let mut c = cfg("avatars");
        c.s3.as_mut().unwrap().public_base_url = None;
        let s = S3Storage::new(&c).unwrap();
        assert_eq!(
            s.public_url("avatars/a.png"),
            "http://127.0.0.1:59000/axum-test/avatars/avatars/a.png"
        );
    }

    /// 空前缀时不能拼出 `//`
    #[test]
    fn an_empty_prefix_does_not_produce_a_double_slash() {
        let s = store("");
        assert_eq!(
            s.public_url("avatars/a.png"),
            "https://cdn.example.com/avatars/a.png"
        );
        assert_eq!(
            s.key_of_url("https://cdn.example.com/avatars/a.png")
                .as_deref(),
            Some("avatars/a.png")
        );
    }

    #[test]
    fn urls_round_trip_back_to_their_logical_key() {
        let s = store("avatars");
        let url = s.public_url("avatars/a.png");
        assert_eq!(
            s.key_of_url(&url).as_deref(),
            Some("avatars/a.png"),
            "删除旧头像全靠这一步反解 key，反解不出来就等于删不掉"
        );
    }

    /// 反解不出来时必须返回 None 而不是"猜一个"——猜错就是删掉别人的对象
    #[test]
    fn foreign_urls_are_not_claimed() {
        let s = store("avatars");
        for hostile in [
            "https://evil.example.com/avatars/avatars/a.png",
            "/uploads/avatars/a.png",
            "https://cdn.example.com/other/a.png",
            "https://cdn.example.com/avatars/../../etc/passwd",
            "https://cdn.example.com/",
        ] {
            assert_eq!(
                s.key_of_url(hostile),
                None,
                "{hostile} 不该被当成本后端的对象"
            );
        }
    }

    #[tokio::test]
    async fn put_refuses_an_empty_payload_without_touching_the_bucket() {
        let s = store("avatars");
        assert!(s.put("image/png", b"").await.is_err());
        assert!(s.put("image/svg+xml", b"<svg/>").await.is_err());
    }
}
