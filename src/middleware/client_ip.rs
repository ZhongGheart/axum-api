//! 客户端 IP 解析
//!
//! 转发头（`X-Forwarded-For` / `X-Real-IP`）是客户端可伪造的输入，
//! 只有当部署方显式声明"本服务位于可信代理之后"（`TRUST_PROXY_HEADERS=true`）
//! 才采信它们；否则使用 TCP 连接的真实来源地址。
//!
//! 限流与登录失败计数都以该结果为键，取错会导致限流被轻易绕过。

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::request::Parts;

/// 已解析的客户端 IP，由限流中间件注入请求扩展
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<ClientIp>()
            .cloned()
            .unwrap_or_else(|| ClientIp("unknown".to_string())))
    }
}

/// 解析客户端 IP
pub fn resolve_client_ip(req: &Request, trust_proxy_headers: bool) -> String {
    if trust_proxy_headers {
        if let Some(ip) = forwarded_ip(req) {
            return ip;
        }
    }

    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }

    tracing::debug!("无法解析客户端 IP（缺少 ConnectInfo 且未信任转发头）");
    "unknown".to_string()
}

/// 从转发头提取第一个合法 IP
fn forwarded_ip(req: &Request) -> Option<String> {
    if let Some(raw) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(ip) = raw
            .split(',')
            .next()
            .and_then(|first| first.trim().parse::<IpAddr>().ok())
        {
            return Some(ip.to_string());
        }
    }

    req.headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
}
