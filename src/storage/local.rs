//! 本地磁盘后端
//!
//! 这是 v0.20.0 起的既有行为，逐字保留：**默认后端**，
//! 未设置 `STORAGE_BACKEND` 时走的仍是这条路径。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use super::{avatar_key, content_type_for, LoadedObject, SavedObject, Storage, UPLOAD_URL_PREFIX};
use crate::config::StorageConfig;
use crate::error::AppError;

#[derive(Debug)]
pub struct LocalStorage {
    root: String,
    max_file_size: usize,
    allowed_mime_types: Vec<String>,
}

impl LocalStorage {
    /// 构造并确保落盘目录存在
    ///
    /// 目录缺失必须在**启动期**暴露：`ServeDir` 只在请求到达时才解析路径，
    /// 不在这里建目录的话，"部署漏了挂卷"要等到第一个用户传头像时才被发现。
    pub fn new(config: &StorageConfig) -> Result<Self, AppError> {
        let root = Path::new(&config.dir).join("avatars");
        std::fs::create_dir_all(&root).map_err(|e| {
            AppError::InternalServerError(format!("创建上传目录 {} 失败: {e}", root.display()))
        })?;
        Ok(Self {
            root: root.to_string_lossy().to_string(),
            max_file_size: config.max_file_size,
            allowed_mime_types: config.allowed_mime_types.clone(),
        })
    }

    /// 把 key 解析成绝对路径，并确认它没跳出上传目录
    ///
    /// **必须有这一步**：传进来的是数据库里的历史值，
    /// 而历史数据、外部导入、或将来某个管理端点都可能让那个值不是本模块生成的。
    /// 不校验就删/读，等于把"删自己的头像"变成任意文件读取器与删除器。
    fn resolve(&self, key: &str) -> Option<PathBuf> {
        let name = key.strip_prefix("avatars/")?;
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return None;
        }
        Some(Path::new(&self.root).join(name))
    }
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(&self, mime: &str, bytes: &[u8]) -> Result<SavedObject, AppError> {
        // 再确认一次体积：调用方可能来自不同的读取路径，
        // 而"类型过了、体积没过"这种组合必须在这里被拦下，而不是写满磁盘才发现。
        if bytes.len() > self.max_file_size {
            return Err(AppError::PayloadTooLarge(format!(
                "图片不超过 {} 字节，当前 {}",
                self.max_file_size,
                bytes.len()
            )));
        }
        if bytes.is_empty() {
            return Err(AppError::BadRequest("图片内容为空".into()));
        }
        if !self.allowed_mime_types.iter().any(|m| m == mime) {
            return Err(AppError::BadRequest(format!(
                "不支持的图片类型：{mime}（仅接受 JPEG / PNG / WebP / GIF）"
            )));
        }
        let key = avatar_key(mime)?;
        let path = self
            .resolve(&key)
            .ok_or_else(|| AppError::InternalServerError("生成的 key 越界".into()))?;

        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| AppError::InternalServerError(format!("创建文件失败: {e}")))?;
        file.write_all(bytes)
            .await
            .map_err(|e| AppError::InternalServerError(format!("写入文件失败: {e}")))?;
        // 不做 fsync：头像丢了是可接受的（ROADMAP 已判定），
        // 但 flush 失败必须报出来，否则"上传成功"与实际可见状态可能不一致。
        file.flush()
            .await
            .map_err(|e| AppError::InternalServerError(format!("刷新文件缓冲失败: {e}")))?;

        Ok(SavedObject {
            url: self.public_url(&key),
            key,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        let Some(path) = self.resolve(key) else {
            tracing::info!("头像路径不在上传目录内，跳过删除: {key}");
            return Ok(());
        };
        if tokio::fs::try_exists(&path)
            .await
            .map_err(|e| AppError::InternalServerError(format!("检查头像文件失败: {e}")))?
        {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| AppError::InternalServerError(format!("删除旧头像失败: {e}")))?;
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<LoadedObject>, AppError> {
        let Some(path) = self.resolve(key) else {
            return Ok(None);
        };
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(LoadedObject {
                bytes,
                content_type: content_type_for(key),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::InternalServerError(format!(
                "读取头像文件失败: {e}"
            ))),
        }
    }

    fn public_url(&self, key: &str) -> String {
        format!("{UPLOAD_URL_PREFIX}/{key}")
    }

    fn key_of_url(&self, url: &str) -> Option<String> {
        super::key_of_url_under(url, UPLOAD_URL_PREFIX, "")
    }

    fn backend_name(&self) -> &'static str {
        "local"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &str) -> StorageConfig {
        StorageConfig {
            backend: crate::config::StorageBackend::Local,
            dir: dir.to_string(),
            max_file_size: 1024,
            allowed_mime_types: ["image/jpeg", "image/png", "image/webp", "image/gif"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            s3: None,
        }
    }

    fn tmp_dir(tag: &str) -> String {
        format!(
            "{}/upload-test-{tag}-{}",
            std::env::temp_dir().display(),
            uuid::Uuid::new_v4().simple()
        )
    }

    async fn store(dir: &str) -> LocalStorage {
        LocalStorage::new(&cfg(dir)).unwrap()
    }

    #[tokio::test]
    async fn only_whitelisted_mime_types_are_accepted() {
        let dir = tmp_dir("mime");
        let s = store(&dir).await;
        assert!(s.put("image/svg+xml", b"<svg/>").await.is_err());
        assert!(s.put("application/pdf", b"%PDF").await.is_err());
        assert!(s.put("image/png", b"\x89PNG").await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn oversized_and_empty_payloads_are_refused() {
        let dir = tmp_dir("size");
        let s = store(&dir).await;
        let big = vec![b'x'; 2048];
        assert!(s.put("image/png", &big).await.is_err());
        assert!(s.put("image/png", b"").await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 存量行为不许变：URL 必须是站内相对路径
    #[tokio::test]
    async fn the_url_stays_a_relative_path_just_like_before_d2() {
        let dir = tmp_dir("url");
        let s = store(&dir).await;
        let saved = s.put("image/png", b"\x89PNG").await.unwrap();
        assert!(
            saved.url.starts_with("/uploads/avatars/"),
            "本地后端的 URL 必须仍是 /uploads/avatars/ 开头: {}",
            saved.url
        );
        assert!(!saved.url.contains(".."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn deletion_refuses_paths_outside_the_avatar_directory() {
        let dir = tmp_dir("del");
        let s = store(&dir).await;
        let saved = s.put("image/png", b"\x89PNG").await.unwrap();
        assert!(s.get(&saved.key).await.unwrap().is_some());

        // 站内但不是本模块生成的路径：跳过而不是误删
        for hostile in [
            "avatars/../../etc/passwd",
            "avatars/sub/dir.png",
            "etc/passwd",
            "../avatars/x.png",
        ] {
            s.delete(hostile).await.unwrap();
            assert!(
                s.get(&saved.key).await.unwrap().is_some(),
                "{hostile} 不得触发删除"
            );
        }

        s.delete(&saved.key).await.unwrap();
        assert!(s.get(&saved.key).await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 删除一个已经不在的文件必须是成功，否则"重复删旧头像"会把上传报成失败
    #[tokio::test]
    async fn deleting_a_missing_object_succeeds() {
        let dir = tmp_dir("missing");
        let s = store(&dir).await;
        assert!(s.delete("avatars/does-not-exist.png").await.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn round_trip_returns_the_exact_bytes_and_content_type() {
        let dir = tmp_dir("roundtrip");
        let s = store(&dir).await;
        let bytes = vec![1u8, 2, 3, 4, 5];
        let saved = s.put("image/webp", &bytes).await.unwrap();
        let loaded = s.get(&saved.key).await.unwrap().unwrap();
        assert_eq!(loaded.bytes, bytes, "回读字节必须与写入一致");
        assert_eq!(loaded.content_type, "image/webp");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
