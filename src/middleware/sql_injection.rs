//! SQL 注入防护中间件
//!
//! 在 HTTP 请求到达路由器之前过滤危险关键词。
//! 虽然 SQLx 的参数化查询已经杜绝了 SQL 注入风险，
//! 此中间件增加深度防御层防护。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// SQL 注入防护配置
#[derive(Debug, Clone)]
pub struct SqlInjectionConfig {
    /// 是否启用
    pub enabled: bool,
    /// 危险关键词列表
    pub dangerous_keywords: Vec<String>,
    /// 需要过滤的路径前缀（仅限定路径生效）
    pub filtered_paths: Vec<String>,
}

impl Default for SqlInjectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dangerous_keywords: vec![
                // SQL 关键字
                "--".to_string(), "/*".to_string(), "*/".to_string(),
                " UNION ".to_string(), " OR 1=1".to_string(), " AND 1=1".to_string(),
                " DROP ".to_string(), " DELETE ".to_string(), " TRUNCATE ".to_string(),
                " EXEC ".to_string(), " EXECUTE ".to_string(), " DECLARE ".to_string(),
                " WAITFOR ".to_string(), " BENCHMARK ".to_string(), " SLEEP(".to_string(),
                " pg_sleep".to_string(), " SHUTDOWN ".to_string(),
                // XSS 关键词
                "<script".to_string(), "javascript:".to_string(), "onerror=".to_string(),
            ],
            filtered_paths: vec![
                "/api/auth/login".to_string(),
                "/api/auth/register".to_string(),
                "/api/admin".to_string(),
            ],
        }
    }
}

/// SQL 注入防护状态
pub type SqlInjectionState = Arc<RwLock<SqlInjectionConfig>>;

/// 创建默认状态
pub fn create_sql_injection_state() -> SqlInjectionState {
    Arc::new(RwLock::new(SqlInjectionConfig::default()))
}

/// SQL 注入防护中间件
///
/// 检查请求参数体（JSON）中是否包含危险关键词。
/// 参数化查询已完全杜绝 SQL 注入，此中间件作为深度防御。
pub async fn sql_injection_middleware(
    State(state): State<SqlInjectionState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let config = state.read().await;
    if !config.enabled {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path().to_string();

    // 检查路径是否需要过滤
    let should_filter = config.filtered_paths.iter().any(|p| path.starts_with(p));
    if !should_filter {
        return Ok(next.run(req).await);
    }

    // 检查 Content-Type
    let content_type = req
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("application/json") {
        return Ok(next.run(req).await);
    }

    // 仅对 POST/PUT/PATCH 请求体进行检查
    let method = req.method().as_str();
    if !["POST", "PUT", "PATCH"].contains(&method) {
        return Ok(next.run(req).await);
    }

    // 提取请求体检查
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 100) // 限制 100KB
        .await
        .map_err(|_| {
            let body = Json(json!({
                "code": 400,
                "message": "请求体过大",
                "data": null,
            }));
            (StatusCode::BAD_REQUEST, body).into_response()
        })?;

    let body_str = String::from_utf8_lossy(&bytes);

    // 检查危险关键词
    let upper_body = body_str.to_uppercase();
    for keyword in &config.dangerous_keywords {
        let upper_keyword = keyword.to_uppercase();
        if upper_body.contains(&upper_keyword) {
            tracing::warn!(
                "SQL 注入防护: 检测到危险关键词 '{}' 在路径 {} 的请求体中",
                keyword, path
            );
            let body = Json(json!({
                "code": 400,
                "message": "请求包含非法字符",
                "data": null,
            }));
            return Err((StatusCode::BAD_REQUEST, body).into_response());
        }
    }

    // 重建请求
    let req = Request::from_parts(parts, axum::body::Body::from(bytes));
    Ok(next.run(req).await)
}
