//! 接口限流中间件
//!
//! 基于 Redis 的 INCR + EXPIRE 模式实现滑动窗口限流。
//! 支持 IP 级和用户级限流，配置通过 `.env` 控制。

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::config::RateLimitConfig;
use crate::utils::redis::RedisClient;

/// 限流中间件状态（由 router 传入的元组）
pub type RateLimitState = (Arc<RedisClient>, Arc<RateLimitConfig>);

/// 限流中间件
///
/// 同时检查 IP 级和用户级（如果已认证）限流。
pub async fn rate_limit_middleware(
    State((redis_client, rate_limit_config)): State<RateLimitState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    // 健康检查不得被限流依赖阻断：依赖故障时仍需可探活
    if req.uri().path() == "/api/health" {
        return Ok(next.run(req).await);
    }

    let client_ip = req
        .headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            req.headers()
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or("unknown");

    let ip_result = redis_client
        .check_ip_rate_limit(
            client_ip,
            rate_limit_config.ip_max_requests,
            rate_limit_config.ip_window_seconds,
        )
        .await
        .map_err(|e| {
            tracing::error!("限流依赖不可用（IP 维度）: {e}");
            let body = Json(json!({
                "code": 503,
                "message": "限流服务不可用",
                "data": null,
            }));
            (StatusCode::SERVICE_UNAVAILABLE, body).into_response()
        })?;

    if !ip_result.allowed {
        let body = Json(json!({
            "code": 429,
            "message": format!(
                "请求过于频繁，IP 限流: {}/{} (窗口: {}s)",
                ip_result.current, ip_result.limit, rate_limit_config.ip_window_seconds
            ),
            "data": null,
        }));
        return Err((StatusCode::TOO_MANY_REQUESTS, body).into_response());
    }

    if let Some(auth_user) = req.extensions().get::<crate::middleware::auth::AuthenticatedUser>() {
        let user_result = redis_client
            .check_user_rate_limit(
                &auth_user.user_id.to_string(),
                rate_limit_config.user_max_requests,
                rate_limit_config.user_window_seconds,
            )
            .await
            .map_err(|e| {
                tracing::error!("限流依赖不可用（用户维度）: {e}");
                let body = Json(json!({
                    "code": 503,
                    "message": "限流服务不可用",
                    "data": null,
                }));
                (StatusCode::SERVICE_UNAVAILABLE, body).into_response()
            })?;

        if !user_result.allowed {
            let body = Json(json!({
                "code": 429,
                "message": format!(
                    "请求过于频繁，用户限流: {}/{} (窗口: {}s)",
                    user_result.current, user_result.limit, rate_limit_config.user_window_seconds
                ),
                "data": null,
            }));
            return Err((StatusCode::TOO_MANY_REQUESTS, body).into_response());
        }
    }

    let mut resp = next.run(req).await;

    resp.headers_mut().insert(
        "X-RateLimit-Limit",
        rate_limit_config.ip_max_requests.to_string().parse().unwrap(),
    );

    Ok(resp)
}
