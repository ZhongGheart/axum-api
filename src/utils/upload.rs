//! 头像落盘
//!
//! v0.20.0 把头像放在**本地磁盘**（见 ROADMAP v0.22.0：那里会换成对象存储）。
//! 本模块刻意只做三件事，其余交给调用方：定文件名、限大小与类型、写盘。
//!
//! ── 为什么不直接用上传请求里的文件名 ──
//!
//! 客户端给的文件名是**不可信输入**。`../../etc/cron.d/x`、`a/b.png`、
//! 含 NUL 或超长片段的名字，都能把写入位置挪到上传目录之外。
//! 因此这里的文件名完全由服务端生成：UUID + 从 MIME 推出的扩展名。
//! 原始文件名只用于回显，绝不参与路径拼接。

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::config::UploadConfig;
use crate::error::AppError;

/// 上传根目录在 URL 上的前缀
///
/// 必须与 `validation::normalize_avatar_url` 里的常量保持一致，
/// 否则上传成功的图存不进 `avatar_url` 字段——校验会把它当站外路径拒掉。
pub const UPLOAD_URL_PREFIX: &str = "/uploads";

/// MIME → 扩展名
///
/// 由**白名单里的 MIME** 决定，不是"原文件名的扩展名"。
/// 反过来做（`shell.html` 声称自己是 png 就直接信）等于让攻击者
/// 把任意字节存成 `.png` 再由浏览器去渲染——那是一条完整的存储型攻击链。
fn extension_for(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// 上传结果
#[derive(Debug, Clone)]
pub struct SavedAvatar {
    /// 可直接写进 `avatar_url` 的站内相对路径，如 `/uploads/avatars/xxx.png`
    pub url: String,
    /// 磁盘位置，供替换旧头像时删除
    pub path: PathBuf,
}

/// 保存一个头像文件
pub async fn save_avatar_bytes(
    config: &UploadConfig,
    mime: &str,
    bytes: &[u8],
) -> Result<SavedAvatar, AppError> {
    let ext = extension_for(mime).ok_or_else(|| {
        AppError::BadRequest(format!(
            "不支持的图片类型：{mime}（仅接受 JPEG / PNG / WebP / GIF）"
        ))
    })?;

    // 再确认一次字节数：调用方可能来自不同的读取路径，
    // 而"类型过了、体积没过"这种组合必须在这里被拦下，而不是写满磁盘才发现。
    if bytes.len() > config.max_file_size {
        return Err(AppError::PayloadTooLarge(format!(
            "图片不超过 {} 字节，当前 {}",
            config.max_file_size,
            bytes.len()
        )));
    }
    if bytes.is_empty() {
        return Err(AppError::BadRequest("图片内容为空".into()));
    }

    let dir = Path::new(&config.dir).join("avatars");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建上传目录失败: {e}")))?;

    let filename = format!("{}.{}", Uuid::new_v4().simple(), ext);
    let path = dir.join(&filename);

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

    Ok(SavedAvatar {
        url: format!("{UPLOAD_URL_PREFIX}/avatars/{filename}"),
        path,
    })
}

/// 删除一个由本模块生成过的头像文件
///
/// **必须校验路径前缀**：传进来的是数据库里的 `avatar_url`，
/// 而历史数据、外部导入、或将来某个管理端点都可能让那个字段
/// 不是本模块写的路径。若不校验就删，等于把"删自己的头像"
/// 变成任意文件删除器。
pub async fn delete_uploaded_avatar(config: &UploadConfig, url: &str) -> Result<(), AppError> {
    let root = Path::new(&config.dir).join("avatars");
    let Some(name) = url
        .strip_prefix(&format!("{UPLOAD_URL_PREFIX}/avatars/"))
        .filter(|n| !n.is_empty() && !n.contains('/') && !n.contains(".."))
    else {
        tracing::info!("头像路径不在上传目录内，跳过删除: {url}");
        return Ok(());
    };
    let path = root.join(name);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &str) -> UploadConfig {
        UploadConfig {
            dir: dir.to_string(),
            max_file_size: 1024,
            allowed_mime_types: ["image/jpeg", "image/png", "image/webp", "image/gif"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    fn tmp_dir(tag: &str) -> String {
        format!(
            "{}/upload-test-{tag}-{}",
            std::env::temp_dir().display(),
            uuid::Uuid::new_v4().simple()
        )
    }

    #[tokio::test]
    async fn only_whitelisted_mime_types_are_accepted() {
        let dir = tmp_dir("mime");
        assert!(save_avatar_bytes(&cfg(&dir), "image/svg+xml", b"<svg/>")
            .await
            .is_err());
        assert!(save_avatar_bytes(&cfg(&dir), "application/pdf", b"%PDF")
            .await
            .is_err());
        assert!(save_avatar_bytes(&cfg(&dir), "image/png", b"\x89PNG")
            .await
            .is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn oversized_and_empty_payloads_are_refused() {
        let dir = tmp_dir("size");
        let big = vec![b'x'; 2048];
        assert!(save_avatar_bytes(&cfg(&dir), "image/png", &big)
            .await
            .is_err());
        assert!(save_avatar_bytes(&cfg(&dir), "image/png", b"")
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_saved_name_never_contains_the_client_filename() {
        let dir = tmp_dir("name");
        let saved = save_avatar_bytes(&cfg(&dir), "image/png", b"\x89PNG")
            .await
            .unwrap();
        let file_name = saved
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(file_name.ends_with(".png"));
        assert!(!file_name.contains(".."));
        assert!(!file_name.contains('/'));
        assert!(saved.url.starts_with("/uploads/avatars/"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn deletion_refuses_paths_outside_the_avatar_directory() {
        let dir = tmp_dir("del");
        let saved = save_avatar_bytes(&cfg(&dir), "image/png", b"\x89PNG")
            .await
            .unwrap();
        assert!(saved.path.exists());

        // 站内但不是本模块生成的路径：跳过而不是误删
        delete_uploaded_avatar(&cfg(&dir), "/uploads/avatars/../../etc/passwd")
            .await
            .unwrap();
        assert!(saved.path.exists(), "越界路径不得触发删除");

        delete_uploaded_avatar(&cfg(&dir), &saved.url)
            .await
            .unwrap();
        assert!(!saved.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
