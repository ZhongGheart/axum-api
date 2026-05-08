//! 请求全局 ID 中间件
//!
//! 为每个请求生成唯一 `x-request-id`，注入到请求扩展和响应头中。
//! 同时将该 ID 注入 tracing span，实现全链路日志追踪。

use axum::{
    body::Body,
    extract::Request,
    http::HeaderValue,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::Span;
use uuid::Uuid;

/// 请求 ID 扩展键，供下游代码从 `req.extensions()` 中提取
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// 全局请求 ID 中间件
///
/// 1. 从请求头 `X-Request-Id` 读取（如果存在），否则生成 UUID
/// 2. 注入到请求扩展中
/// 3. 注入到响应头
/// 4. 注入到 tracing span 的 `request_id` 字段
pub async fn request_id_middleware(
    mut req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    // 从请求头读取或生成 UUID
    let request_id = req
        .headers()
        .get("X-Request-Id")
        .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // 注入到扩展
    req.extensions_mut()
        .insert(RequestId(request_id.clone()));

    // 注入到 tracing span
    let span = Span::current();
    span.record("request_id", tracing::field::display(&request_id));

    let mut resp = next.run(req).await;

    // 注入到响应头
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        resp.headers_mut().insert("X-Request-Id", value);
    }

    Ok(resp)
}
