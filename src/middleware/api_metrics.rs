//! API 性能追踪中间件
//!
//! 记录每个接口的响应时间、调用次数、报错次数。
//!
//! ## 为什么落到 Redis（而不是进程内 HashMap）
//!
//! 进程内累计有三个问题，都会在多副本部署下变成错误数据：
//!
//! 1. **重启即丢**：累计值随进程消失
//! 2. **多副本不准**：每个副本只看见自己那份流量，
//!    监控页上的 QPS 实际是"本副本 QPS"
//! 3. **重置不跨副本**：`reset` 只清本进程，别的副本照旧累加
//!
//! 现改为「本地增量缓冲 + 定时 flush 到 Redis」：
//! 请求路径上只做一次内存合并（无网络往返），
//! 后台任务把增量 `HINCRBY` 进 Redis，于是所有副本写同一份计数。
//!
//! **Redis 被 flush 或未持久化时指标会丢**，这对监控数据是可接受的取舍：
//! 换来的是跨副本准确与重启不丢（优雅关闭前会做最后一次 flush）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
};

use crate::config::MetricsConfig;
use crate::utils::redis::RedisClient;

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

/// 待写入 Redis 的本地增量
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MetricsDelta {
    call_count: u64,
    error_count: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
    /// 用 `Option` 而非 0 表示"还没有样本"：
    /// 亚毫秒请求的耗时**真的就是 0**，用 0 当哨兵会被后续更大的值覆盖
    min_duration_ms: Option<u64>,
}

impl MetricsDelta {
    /// 吸收一次调用
    fn absorb(&mut self, duration_ms: u64, is_error: bool) {
        self.call_count += 1;
        self.total_duration_ms += duration_ms;
        self.max_duration_ms = self.max_duration_ms.max(duration_ms);
        self.min_duration_ms = Some(match self.min_duration_ms {
            Some(cur) => cur.min(duration_ms),
            None => duration_ms,
        });
        if is_error {
            self.error_count += 1;
        }
    }

    /// 把 `other` 合并进自己（flush 失败后把增量放回缓冲重试）
    fn merge(&mut self, other: &MetricsDelta) {
        self.call_count += other.call_count;
        self.error_count += other.error_count;
        self.total_duration_ms += other.total_duration_ms;
        self.max_duration_ms = self.max_duration_ms.max(other.max_duration_ms);
        if let Some(other_min) = other.min_duration_ms {
            self.min_duration_ms = Some(match self.min_duration_ms {
                Some(cur) => cur.min(other_min),
                None => other_min,
            });
        }
    }

    /// 转成对外快照
    fn to_metric(&self, method: &str, path: &str) -> EndpointMetric {
        EndpointMetric {
            path: path.to_string(),
            method: method.to_string(),
            call_count: self.call_count,
            error_count: self.error_count,
            total_duration_ms: self.total_duration_ms,
            avg_duration_ms: self.total_duration_ms / self.call_count.max(1),
            max_duration_ms: self.max_duration_ms,
            min_duration_ms: self.min_duration_ms.unwrap_or(0),
        }
    }
}

/// 原子地把一批增量合并进单个端点的 Redis 计数。
///
/// max/min 必须用 Lua 而非读改写：两个副本同时上报时，
/// 读-改-写之间会被另一个副本插入，导致极值被覆盖。
/// Redis 的 Lua 是原子的，因此跨副本也安全。
const MERGE_DELTA_LUA: &str = r#"
redis.call('HINCRBY', KEYS[1], 'c', ARGV[1])
redis.call('HINCRBY', KEYS[1], 'e', ARGV[2])
redis.call('HINCRBY', KEYS[1], 't', ARGV[3])
local cur_max = redis.call('HGET', KEYS[1], 'mx')
if cur_max == false or tonumber(ARGV[4]) > tonumber(cur_max) then
  redis.call('HSET', KEYS[1], 'mx', ARGV[4])
end
local cur_min = redis.call('HGET', KEYS[1], 'mn')
if cur_min == false or tonumber(ARGV[5]) < tonumber(cur_min) then
  redis.call('HSET', KEYS[1], 'mn', ARGV[5])
end
redis.call('EXPIRE', KEYS[1], ARGV[6])
return 1
"#;

/// 指标收集器
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    redis: Arc<RedisClient>,
    cfg: MetricsConfig,
    /// 尚未 flush 的本地增量；键为 `"{method} {path}"`
    ///
    /// 用 `std::sync::Mutex` 而非 `tokio::sync::Mutex`：
    /// 临界区里不做任何 await，可直接同步合并，
    /// 不必再为每个请求 spawn 一个任务
    buffer: Arc<Mutex<HashMap<String, MetricsDelta>>>,
    /// 缓冲溢出告警只发一次，避免 Redis 长期不可用时刷爆日志
    overflow_warned: Arc<AtomicBool>,
}

impl MetricsCollector {
    /// Redis 指标键前缀
    const KEY_PREFIX: &'static str = "metrics:ep:";

    pub fn new(redis: Arc<RedisClient>, cfg: MetricsConfig) -> Self {
        Self {
            redis,
            cfg,
            buffer: Arc::new(Mutex::new(HashMap::new())),
            overflow_warned: Arc::new(AtomicBool::new(false)),
        }
    }

    fn redis_key(method: &str, path: &str) -> String {
        format!("{}{method} {path}", Self::KEY_PREFIX)
    }

    /// 记录一次调用（纯内存，无网络往返）
    pub fn record(&self, method: &str, path: &str, duration_ms: u64, is_error: bool) {
        let key = format!("{method} {path}");
        let mut buffer = match self.buffer.lock() {
            Ok(g) => g,
            // 锁中毒说明上一次持锁时 panic 了；指标可以丢，但不能连带请求
            Err(poisoned) => poisoned.into_inner(),
        };

        // Redis 长期不可用时缓冲会一直涨，兜底封顶并告警
        if !buffer.contains_key(&key) && buffer.len() >= self.cfg.max_buffered_endpoints {
            if !self.overflow_warned.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    max = self.cfg.max_buffered_endpoints,
                    "接口指标本地缓冲已满，新端点的指标将被丢弃（通常是 Redis 长期不可用）"
                );
            }
            return;
        }

        buffer.entry(key).or_default().absorb(duration_ms, is_error);
    }

    /// 把本地增量写入 Redis
    ///
    /// 失败的增量**放回缓冲**等下一轮重试，因此 Redis 短暂抖动不会丢指标。
    pub async fn flush(&self) {
        let taken: HashMap<String, MetricsDelta> = {
            let mut buffer = match self.buffer.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut *buffer)
        };

        if taken.is_empty() {
            return;
        }

        let mut failed: Vec<(String, MetricsDelta)> = Vec::new();
        let mut conn = self.redis.conn.clone();

        for (key, delta) in taken {
            let (method, path) = split_key(&key);
            let min_ms = delta.min_duration_ms.unwrap_or(0);

            let result: Result<i64, redis::RedisError> = redis::Script::new(MERGE_DELTA_LUA)
                .key(Self::redis_key(method, path))
                .arg(delta.call_count)
                .arg(delta.error_count)
                .arg(delta.total_duration_ms)
                .arg(delta.max_duration_ms)
                .arg(min_ms)
                .arg(self.cfg.key_ttl_seconds)
                .invoke_async(&mut conn)
                .await;

            if result.is_err() {
                failed.push((key, delta));
            }
        }

        if failed.is_empty() {
            self.overflow_warned.store(false, Ordering::Relaxed);
            return;
        }

        tracing::warn!(
            端点数 = failed.len(),
            "写入 Redis 指标失败，增量已保留待下轮重试"
        );
        let mut buffer = match self.buffer.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (key, delta) in failed {
            buffer.entry(key).or_default().merge(&delta);
        }
    }

    /// 读取聚合后的指标快照
    ///
    /// Redis 里的累计值会**叠加尚未 flush 的本地增量**，
    /// 免得刚发生的调用最多要等一个 flush 间隔才在页面上出现。
    pub async fn snapshot(&self) -> Vec<EndpointMetric> {
        let mut conn = self.redis.conn.clone();
        let mut aggregated: HashMap<String, MetricsDelta> = HashMap::new();
        let keys = scan_metric_keys(&mut conn).await;

        if !keys.is_empty() {
            // 一次 pipeline 取回全部端点，避免 N 次往返
            let mut pipe = redis::pipe();
            for key in &keys {
                pipe.hgetall(key);
            }
            let rows: Vec<HashMap<String, u64>> =
                pipe.query_async(&mut conn).await.unwrap_or_default();

            for (key, row) in keys.iter().zip(rows) {
                if row.is_empty() {
                    continue;
                }
                aggregated.insert(
                    // 归一化键格式：Redis 键带前缀，本地缓冲键不带。
                    // 不剥掉前缀就插入，同一个端点会变成两行独立记录。
                    normalize_key(key),
                    MetricsDelta {
                        call_count: row.get("c").copied().unwrap_or(0),
                        error_count: row.get("e").copied().unwrap_or(0),
                        total_duration_ms: row.get("t").copied().unwrap_or(0),
                        max_duration_ms: row.get("mx").copied().unwrap_or(0),
                        min_duration_ms: row.get("mn").copied(),
                    },
                );
            }
        }

        // 叠加本地未 flush 的增量
        {
            let buffer = match self.buffer.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (key, delta) in buffer.iter() {
                aggregated.entry(key.clone()).or_default().merge(delta);
            }
        }

        let mut result: Vec<EndpointMetric> = aggregated
            .iter()
            .filter(|(_, d)| d.call_count > 0)
            .map(|(key, d)| {
                let (method, path) = split_key(key);
                d.to_metric(method, path)
            })
            .collect();
        result.sort_by_key(|m| std::cmp::Reverse(m.call_count));
        result
    }

    /// 重置指标：**跨副本**清空
    ///
    /// 此前只清本进程的 HashMap，别的副本仍在累加，
    /// 于是"重置"后监控页的数字仍会立刻涨回来。
    pub async fn reset(&self) {
        {
            let mut buffer = match self.buffer.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            buffer.clear();
        }

        let mut conn = self.redis.conn.clone();
        let keys = scan_metric_keys(&mut conn).await;
        if keys.is_empty() {
            return;
        }

        let mut pipe = redis::pipe();
        for key in &keys {
            pipe.del(key);
        }
        if let Err(e) = pipe.query_async::<()>(&mut conn).await {
            tracing::warn!("重置指标失败，部分 Redis 键可能残留: {e}");
        }
    }

    /// 启动定时 flush 后台任务
    pub fn spawn_flush_task(&self) -> MetricsFlushTask {
        let collector = self.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let interval = std::time::Duration::from_secs(self.cfg.flush_interval_seconds);

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => collector.flush().await,
                    _ = shutdown_rx.changed() => break,
                }
            }
            // 关闭前做最后一次 flush：否则最多一个间隔的指标随进程一起消失
            collector.flush().await;
            tracing::info!("接口指标已全部写入 Redis");
        });

        MetricsFlushTask {
            shutdown: shutdown_tx,
            handle: Some(handle),
        }
    }
}

/// SCAN 出全部指标键。
///
/// 用 SCAN 而非 KEYS：KEYS 会阻塞整个 Redis 实例，共享实例上不可接受。
async fn scan_metric_keys(conn: &mut redis::aio::ConnectionManager) -> Vec<String> {
    let pattern = format!("{}*", MetricsCollector::KEY_PREFIX);
    let mut cursor: u64 = 0;
    let mut keys: Vec<String> = Vec::new();
    loop {
        let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(&pattern)
            .arg("COUNT")
            .arg(200)
            .query_async(conn)
            .await
            .unwrap_or((0, Vec::new()));
        keys.extend(batch);
        cursor = next;
        if cursor == 0 {
            break;
        }
    }
    keys
}

/// 拆回 `("{method}", "{path}")`
///
/// 入参既可能是本地缓冲的 `"{method} {path}"`，
/// 也可能是 Redis 键 `"metrics:ep:{method} {path}"`，前缀需先剥掉——
/// 否则 method 会变成 `"metrics:ep:GET"` 这种脏值。
fn split_key(key: &str) -> (&str, &str) {
    let body = strip_key_prefix(key);
    body.split_once(' ').unwrap_or(("UNKNOWN", body))
}

/// 剥掉 Redis 键前缀，得到统一的 `"{method} {path}"`
fn strip_key_prefix(key: &str) -> &str {
    key.strip_prefix(MetricsCollector::KEY_PREFIX)
        .unwrap_or(key)
}

/// 归一化键格式，供 `snapshot` 合并 Redis 与本地缓冲时使用
fn normalize_key(key: &str) -> String {
    strip_key_prefix(key).to_string()
}

/// 定时 flush 任务的句柄；`shutdown` 会等最后一次 flush 落库
pub struct MetricsFlushTask {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl MetricsFlushTask {
    /// 通知后台任务收尾，并等待它完成最后一次 flush
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
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
    // 用**路由模板**而非原始路径：`/api/admin/users/{id}` 只记成一个端点。
    // 若记原始路径，每个资源 ID 都是独立键——基数无界，
    // 且每个端点只出现一次、count=1，监控页反而看不出真实 QPS。
    // 未匹配任何路由（404）时没有 MatchedPath，回退到原始路径，
    // 否则所有 404 会塌成同一个键。
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let is_error = response.status().is_server_error() || response.status().is_client_error();
    // 记录是纯内存合并，留在请求内同步完成：
    // 不再为每个请求 spawn 一个任务（原先只因为要 await 锁才必须 spawn）
    state
        .metrics_collector
        .record(&method, &path, duration_ms, is_error);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 亚毫秒请求的耗时真的就是 0。
    /// 旧实现拿 0 当"还没有样本"的哨兵，于是 0 会被后续更大的值覆盖，
    /// 最快的那次调用在监控页上凭空消失。
    #[test]
    fn a_zero_duration_is_a_real_minimum_not_a_missing_value() {
        let mut delta = MetricsDelta::default();
        delta.absorb(0, false);
        delta.absorb(50, false);

        assert_eq!(delta.min_duration_ms, Some(0));
        assert_eq!(delta.max_duration_ms, 50);
        assert_eq!(delta.call_count, 2);
    }

    /// 只有一个样本时，min 与 max 必须都是它自己
    #[test]
    fn a_single_sample_is_its_own_min_and_max() {
        let mut delta = MetricsDelta::default();
        delta.absorb(7, true);

        assert_eq!(delta.min_duration_ms, Some(7));
        assert_eq!(delta.max_duration_ms, 7);
        assert_eq!(delta.error_count, 1);
    }

    /// flush 失败后把增量放回缓冲，两批合并的语义必须与一次性记录一致
    #[test]
    fn merging_two_batches_matches_recording_them_in_one_go() {
        let mut first = MetricsDelta::default();
        first.absorb(10, false);
        first.absorb(90, true);

        let mut second = MetricsDelta::default();
        second.absorb(30, false);
        second.absorb(0, false);

        let mut merged = first.clone();
        merged.merge(&second);

        let mut direct = MetricsDelta::default();
        for (d, e) in [(10, false), (90, true), (30, false), (0, false)] {
            direct.absorb(d, e);
        }

        assert_eq!(merged, direct);
        assert_eq!(merged.call_count, 4);
        assert_eq!(merged.error_count, 1);
        assert_eq!(merged.total_duration_ms, 130);
        assert_eq!(merged.min_duration_ms, Some(0));
        assert_eq!(merged.max_duration_ms, 90);
    }

    /// 合并一个还没有任何样本的增量，不能凭空造出 min
    #[test]
    fn merging_an_empty_delta_does_not_invent_a_minimum() {
        let mut delta = MetricsDelta::default();
        delta.absorb(12, false);

        delta.merge(&MetricsDelta::default());

        assert_eq!(delta.min_duration_ms, Some(12));
        assert_eq!(delta.call_count, 1);
    }

    #[test]
    fn avg_duration_does_not_divide_by_zero() {
        let metric = MetricsDelta::default().to_metric("GET", "/api/health");
        assert_eq!(metric.avg_duration_ms, 0);
        assert_eq!(metric.min_duration_ms, 0);
    }

    #[test]
    fn a_metric_key_splits_back_into_method_and_path() {
        assert_eq!(
            split_key("GET /api/admin/users/{id}"),
            ("GET", "/api/admin/users/{id}")
        );
        // Redis 键带 `metrics:ep:` 前缀，前缀必须被剥掉，
        // 否则 method 会变成 "metrics:ep:GET" 这种脏值
        assert_eq!(
            split_key("metrics:ep:GET /api/admin/users/{id}"),
            ("GET", "/api/admin/users/{id}")
        );
        // 没有空格时不应 panic
        assert_eq!(split_key("garbage"), ("UNKNOWN", "garbage"));
    }
}
