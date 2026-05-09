//! API 性能追踪中间件
//!
//! 记录每个接口的响应时间、调用次数、报错次数。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use axum::{extract::{Request, State}, middleware::Next, response::Response};
use tokio::sync::RwLock;

/// 单接口指标快照
#[derive(Debug, Clone, serde::Serialize)]
pub struct EndpointMetric {
    pub path: String,
    pub method: String,
    pub call_count: u64,
    pub error_count: u64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub max_duration_ms: u64,
    pub min_duration_ms: u64,
}

/// 内部指标数据
#[derive(Debug, Clone, Default)]
struct MetricsData {
    call_count: u64,
    error_count: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
    min_duration_ms: u64,
}

/// 指标收集器
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    inner: Arc<RwLock<HashMap<String, MetricsData>>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn record(&self, method: &str, path: &str, duration_ms: u64, is_error: bool) {
        let key = format!("{} {}", method, path);
        let mut map = self.inner.write().await;
        let entry = map.entry(key).or_default();
        entry.call_count += 1;
        entry.total_duration_ms += duration_ms;
        if duration_ms > entry.max_duration_ms {
            entry.max_duration_ms = duration_ms;
        }
        if entry.min_duration_ms == 0 || duration_ms < entry.min_duration_ms {
            entry.min_duration_ms = duration_ms;
        }
        if is_error {
            entry.error_count += 1;
        }
    }

    pub async fn snapshot(&self) -> Vec<EndpointMetric> {
        let map = self.inner.read().await;
        let mut result: Vec<EndpointMetric> = map.iter().map(|(key, m)| {
            let parts: Vec<&str> = key.splitn(2, ' ').collect();
            let method = parts.first().unwrap_or(&"UNKNOWN").to_string();
            let path = parts.get(1).unwrap_or(&"unknown").to_string();
            EndpointMetric {
                path, method,
                call_count: m.call_count,
                error_count: m.error_count,
                total_duration_ms: m.total_duration_ms,
                avg_duration_ms: if m.call_count > 0 { m.total_duration_ms / m.call_count } else { 0 },
                max_duration_ms: m.max_duration_ms,
                min_duration_ms: m.min_duration_ms,
            }
        }).collect();
        result.sort_by(|a, b| b.call_count.cmp(&a.call_count));
        result
    }

    pub async fn reset(&self) {
        self.inner.write().await.clear();
    }
}

/// API 性能追踪中间件
///
/// 从 AppState 中提取 metrics_collector，避免 Axum 类型不匹配。
pub async fn api_metrics_mw(
    State(state): State<crate::router::AppState>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let is_error = response.status().is_server_error() || response.status().is_client_error();
    let mc = state.metrics_collector.clone();
    tokio::spawn(async move {
        mc.record(&method, &path, duration_ms, is_error).await;
    });
    response
}
