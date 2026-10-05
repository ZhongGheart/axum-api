//! 存储抽象（v0.27.0 / ROADMAP D2）
//!
//! v0.20.0 把头像写死在本地磁盘的 `UPLOAD_DIR`，本模块把那套逻辑收进
//! [`Storage`] trait，换后端时**调用方一行不改**。
//!
//! ── 为什么 object key 与对外 URL 必须分开 ──
//!
//! 库里存的是 **URL**，对象存储里放的是 **key**。两者不是一回事：
//! 本地后端 `key = avatars/x.png` 对应 `url = /uploads/avatars/x.png`；
//! S3 后端同一个 `key` 可能对应 CDN 上的绝对地址。
//!
//! 若只存 key，就得再加一个"当前后端是哪个"的运行时判断才能把历史数据渲染出来；
//! 若只把 URL 交给存储层，删除时又得从 URL 反解 key（要重新做一遍前缀解析，
//! 也就是把 v0.20.0 里那个"必须校验路径前缀"的防护再写一次，且这次是手写正则）。
//! 分开之后，删除永远按 key 走，**URL 只用于展示**。

pub mod local;
pub mod s3;

use async_trait::async_trait;

use crate::config::{StorageBackend, StorageConfig};
use crate::error::AppError;

/// 上传目录在 URL 上的前缀
///
/// 必须与 `validation::normalize_avatar_url` 里的常量保持一致，
/// 否则上传成功的图存不进 `avatar_url` 字段——校验会把它当站外路径拒掉。
pub const UPLOAD_URL_PREFIX: &str = "/uploads";

/// 存进去的对象
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedObject {
    /// 对象存储里的 key，如 `avatars/1a2b.png`。**删除时用这个**，不碰 URL。
    pub key: String,
    /// 可直接写进 `avatar_url` 的地址。站内相对路径或绝对 URL，取决于后端。
    pub url: String,
}

/// 读回来的对象
#[derive(Debug, Clone)]
pub struct LoadedObject {
    pub bytes: Vec<u8>,
    /// 由 key 的扩展名推出，见 [`content_type_for`]
    pub content_type: &'static str,
}

/// 存储后端
///
/// `Debug` 是**超 trait** 而不是可有可无的便利：`AppState` derive 了 `Debug`，
/// 后端不实现它就编译不过。与其给 `AppState` 写一套手写 `Debug`（那样每加一个
/// 字段都要记得同步），不如要求后端自己实现——`S3Storage` 的实现刻意不打凭据。
#[async_trait]
pub trait Storage: Send + Sync + std::fmt::Debug {
    /// 存一个头像，返回 key 与对外 URL
    ///
    /// MIME 白名单与体积上限在这里**再查一遍**，不是重复劳动：
    /// 调用方可能来自不同读取路径，而"类型过了、体积没过"这种组合
    /// 必须在这里被拦下，而不是写满磁盘（或塞满 bucket）才发现。
    async fn put(&self, mime: &str, bytes: &[u8]) -> Result<SavedObject, AppError>;

    /// 按 key 删除。不存在时**静默成功**
    ///
    /// 替换头像时旧文件可能已被并发请求删掉，让"已经没了"变成错误
    /// 只会把一次成功上传报成失败。
    async fn delete(&self, key: &str) -> Result<(), AppError>;

    /// 按 key 读回对象，不存在返回 `Ok(None)`
    ///
    /// S3 后端在 bucket 私有时由本进程代理读（见 router），
    /// 本地后端用不到这个方法——它由 `ServeDir` 直接服务。
    async fn get(&self, key: &str) -> Result<Option<LoadedObject>, AppError>;

    /// 本后端把 key 渲染成什么 URL
    fn public_url(&self, key: &str) -> String;

    /// 从库里存的 URL 反解出 key，用于删除旧对象
    ///
    /// **返回 `None` 表示"这不是本后端的东西"，调用方应跳过删除。**
    /// 绝不返回"猜出来的"key——猜错就是删掉别人的对象。
    ///
    /// 为什么需要它：库里存的是 URL，而 URL 的形态取决于**当初上传时**
    /// 用的是哪个后端。v0.27.0 之后同一个库里可能同时躺着
    /// `/uploads/avatars/x.png`（本地时期）与
    /// `https://cdn/.../x.png`（S3 时期）两种值，删除时必须问后端本人。
    fn key_of_url(&self, url: &str) -> Option<String>;

    /// 后端名，进启动日志与测试断言
    fn backend_name(&self) -> &'static str;
}

/// MIME → 扩展名
///
/// 由**白名单里的 MIME** 决定，不是"原文件名的扩展名"。
/// 反过来做（`shell.html` 声称自己是 png 就直接信）等于让攻击者
/// 把任意字节存成 `.png` 再由浏览器去渲染——那是一条完整的存储型攻击链。
pub fn extension_for(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// 扩展名 → Content-Type
///
/// S3 后端代理读对象时需要回 `Content-Type`，否则浏览器按
/// `application/octet-stream` 处理，头像点了变成下载而不是显示。
/// 未知扩展名一律 `application/octet-stream`：宁可让浏览器存下来，
/// 也不要谎称是图片——key 只由本模块生成，走不到这条分支。
pub fn content_type_for(key: &str) -> &'static str {
    match key.rsplit('.').next().unwrap_or_default() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// 生成一个头像 key
///
/// 文件名**完全由服务端生成**（UUID + 从 MIME 推出的扩展名）。
/// 客户端给的文件名是**不可信输入**：`../../etc/cron.d/x`、`a/b.png`、
/// 含 NUL 或超长片段的名字，都能把写入位置挪到上传目录之外。
/// 原始文件名只用于回显，绝不参与 key 拼接。
pub fn avatar_key(mime: &str) -> Result<String, AppError> {
    let ext = extension_for(mime).ok_or_else(|| {
        AppError::BadRequest(format!(
            "不支持的图片类型：{mime}（仅接受 JPEG / PNG / WebP / GIF）"
        ))
    })?;
    Ok(format!("avatars/{}.{ext}", uuid::Uuid::new_v4().simple()))
}

/// 两个后端共用的"URL → key"反解
///
/// 抽出来是因为本地与 S3 的差别只是 base 和 prefix 的取值，
/// 而**校验部分必须一模一样**——它是删除路径上唯一的越权防线，
/// 两份实现里任何一份漏掉 `..` 检查就是一个任意对象删除器。
pub(crate) fn key_of_url_under(url: &str, base: &str, prefix: &str) -> Option<String> {
    let full = url.strip_prefix(base)?.strip_prefix('/')?;
    let key = if prefix.is_empty() {
        full
    } else {
        full.strip_prefix(prefix)?.strip_prefix('/')?
    };
    // key 只由 `avatar_key` 生成，形状恒为 `avatars/<uuid>.<ext>`。
    // 但传进来的是**库里的历史值**，所以这里按形状收紧：
    // 任一段是 `..`、或段数超过两层，都不是本模块生成的东西。
    let mut segments = key.split('/');
    let dir = segments.next()?;
    let name = segments.next()?;
    if segments.next().is_some() || dir != "avatars" || name.is_empty() {
        return None;
    }
    if name.contains("..") || name.contains('\\') || name.contains('\0') {
        return None;
    }
    Some(key.to_string())
}

/// 按配置构建后端
pub fn build(config: &StorageConfig) -> Result<std::sync::Arc<dyn Storage>, AppError> {
    Ok(match config.backend {
        StorageBackend::Local => std::sync::Arc::new(local::LocalStorage::new(config)?),
        StorageBackend::S3 => std::sync::Arc::new(s3::S3Storage::new(config)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_derived_from_the_mime_never_from_the_client() {
        let key = avatar_key("image/png").unwrap();
        assert!(
            key.starts_with("avatars/"),
            "key 必须带 avatars/ 前缀: {key}"
        );
        assert!(key.ends_with(".png"), "扩展名必须来自 MIME: {key}");
        assert_eq!(key.matches('/').count(), 1, "key 只有一层目录: {key}");
        assert!(!key.contains(".."), "key 里绝不能出现 ..: {key}");
        // 两次生成必须不同，否则后一次上传会覆盖前一次
        assert_ne!(key, avatar_key("image/png").unwrap());
    }

    #[test]
    fn mime_outside_the_whitelist_has_no_extension() {
        for mime in ["image/svg+xml", "application/pdf", "text/html", ""] {
            assert!(
                avatar_key(mime).is_err(),
                "{mime} 不该拿到 key，否则等于放行一条存储型 XSS 链"
            );
        }
    }

    /// 扩展名 → Content-Type 必须与生成 key 的那张表互为逆映射，
    /// 否则 S3 代理读回来的头像会被浏览器当二进制下载
    #[test]
    fn content_type_is_the_inverse_of_extension_for() {
        for mime in ["image/jpeg", "image/png", "image/webp", "image/gif"] {
            let key = avatar_key(mime).unwrap();
            assert_eq!(
                content_type_for(&key),
                mime,
                "{mime} 存下来再读出来必须是同一个 Content-Type"
            );
        }
        assert_eq!(
            content_type_for("avatars/x.txt"),
            "application/octet-stream"
        );
        assert_eq!(content_type_for("avatars/x"), "application/octet-stream");
    }

    /// 删除路径上唯一的越权防线。传进来的是**库里的历史值**，
    /// 所以这里必须按"只有本模块生成的东西才认"的尺度收紧。
    #[test]
    fn url_to_key_only_claims_what_this_module_generated() {
        let base = "https://cdn.example.com";
        assert_eq!(
            key_of_url_under("https://cdn.example.com/axum/avatars/a.png", base, "axum").as_deref(),
            Some("avatars/a.png")
        );
        assert_eq!(
            key_of_url_under("/uploads/avatars/a.png", "/uploads", "").as_deref(),
            Some("avatars/a.png")
        );

        for hostile in [
            // 别人的域名
            "https://evil.example.com/axum/avatars/a.png",
            // base 前缀相同但多一个字符
            "https://cdn.example.com.evil.net/axum/avatars/a.png",
            // 前缀目录不对
            "https://cdn.example.com/other/avatars/a.png",
            // 多一层目录
            "https://cdn.example.com/axum/avatars/sub/a.png",
            // key 位置放 ..
            "https://cdn.example.com/axum/../a.png",
            "https://cdn.example.com/axum/avatars/..%2Fa.png",
            // 没有文件名
            "https://cdn.example.com/axum/avatars/",
            "https://cdn.example.com/axum/",
            // 目录名不对
            "https://cdn.example.com/axum/notavatars/a.png",
            // 本地后端不该认 S3 地址，反之亦然
            "/uploads/axum/avatars/a.png",
        ] {
            assert_eq!(
                key_of_url_under(hostile, base, "axum"),
                None,
                "{hostile} 不该被当成本后端的对象"
            );
        }
    }

    /// 反解必须与 `public_url` 互逆，否则上传换头像时旧对象永远删不掉
    #[test]
    fn url_to_key_is_the_inverse_of_public_url() {
        for (base, prefix) in [
            ("/uploads", ""),
            ("https://cdn.example.com", "avatars"),
            ("https://cdn.example.com", "axum/prod"),
        ] {
            let key = avatar_key("image/png").unwrap();
            let url = if prefix.is_empty() {
                format!("{base}/{key}")
            } else {
                format!("{base}/{prefix}/{key}")
            };
            assert_eq!(
                key_of_url_under(&url, base, prefix).as_deref(),
                Some(key.as_str()),
                "base={base} prefix={prefix} 的一轮往返必须回到原 key"
            );
        }
    }
}
