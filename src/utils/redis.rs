//! Redis 通用工具模块
//!
//! 封装异步 Redis 客户端，提供连接管理、Token 黑名单、限流计数等通用能力。

use std::time::Duration;

use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use redis::AsyncCommands;

use crate::config::RedisConfig;

/// Redis 操作结果
pub type Result<T> = std::result::Result<T, crate::error::AppError>;

/// 限流检查结果
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    /// 是否允许通过
    pub allowed: bool,
    /// 当前窗口内已用次数
    pub current: u64,
    /// 窗口上限
    pub limit: u64,
    /// 剩余可请求次数
    pub remaining: u64,
}

/// 通用 Redis 客户端封装
// ConnectionManager 未实现 Debug，手动实现
#[derive(Clone)]
pub struct RedisClient {
    /// 异步连接管理器（自动重连）
    pub conn: ConnectionManager,
}

impl std::fmt::Debug for RedisClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisClient").finish()
    }
}

/// Redis 连接与操作的时间上限
///
/// redis-rs 默认 `response_timeout = None`：Redis 不可用时操作会一直等待重连，
/// 使所有经过限流/黑名单校验的请求挂起。这里给出明确上限，
/// 让依赖故障快速以 503 暴露，而不是拖垮整个服务。
fn redis_manager_config() -> ConnectionManagerConfig {
    ConnectionManagerConfig::new()
        .set_number_of_retries(3)
        .set_factor(50)
        .set_exponent_base(2)
        .set_max_delay(200)
        .set_connection_timeout(Duration::from_secs(1))
        .set_response_timeout(Duration::from_secs(2))
}

impl RedisClient {
    /// 从配置创建 Redis 客户端连接
    pub async fn new(config: &RedisConfig) -> Result<Self> {
        let client = redis::Client::open(config.url.as_str())
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 连接失败: {e}")))?;

        let conn = ConnectionManager::new_with_config(client, redis_manager_config())
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 连接管理初始化失败: {e}")))?;

        Ok(Self { conn })
    }

    /// 健康检查：向 Redis 发送 PING
    pub async fn ping(&self) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                crate::error::AppError::InternalServerError(format!("Redis PING 失败: {e}"))
            })?;
        Ok(())
    }

    // ──────────────────────────────────────────────
    // 会话注销
    // ──────────────────────────────────────────────
    //
    // 两类注销语义：
    // 1. 单令牌注销（登出）：以 jti 为键，只影响当前设备
    // 2. 全量会话吊销（改密/停用/删除账号）：记录时间点，使该用户在此之前签发的令牌全部失效

    /// 单令牌黑名单 Key 前缀
    const TOKEN_BLACKLIST_PREFIX: &'static str = "token:blacklist:";
    /// 用户会话吊销时间点 Key 前缀
    const USER_REVOKED_PREFIX: &'static str = "user:revoked_before:";

    /// 注销单个令牌（jti），TTL 由令牌剩余有效期决定
    pub async fn add_token_to_blacklist(&self, jti: &str, token_exp: u64) -> Result<()> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, jti);
        let now = chrono::Utc::now().timestamp() as u64;
        // 剩余有效秒数 = 令牌过期时间 - 当前时间，最少 1 秒
        let ttl = if token_exp > now { token_exp - now } else { 1 };

        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(key, "1", ttl)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}")))?;

        Ok(())
    }

    /// 查询令牌是否已被注销
    pub async fn is_token_blacklisted(&self, jti: &str) -> Result<bool> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, jti);
        let mut conn = self.conn.clone();
        let exists: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}")))?;
        Ok(exists.is_some())
    }

    /// 吊销某用户当前及之前签发的全部令牌
    ///
    /// 记录"吊销时间点"，`iat` 早于该时间点的令牌一律失效。
    /// 新登录签发的令牌 `iat` 不早于该时间点，因此不会被误伤。
    /// TTL 取令牌最长有效期，过期后键自动清理。
    pub async fn revoke_user_sessions(&self, user_id: &uuid::Uuid, ttl_seconds: u64) -> Result<u64> {
        let key = format!("{}{}", Self::USER_REVOKED_PREFIX, user_id);
        let revoked_before = chrono::Utc::now().timestamp() as u64;
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(key, revoked_before, ttl_seconds.max(1))
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}")))?;
        Ok(revoked_before)
    }

    /// 读取用户会话吊销时间点；未吊销返回 `None`
    pub async fn user_revoked_before(&self, user_id: &uuid::Uuid) -> Result<Option<u64>> {
        let key = format!("{}{}", Self::USER_REVOKED_PREFIX, user_id);
        let mut conn = self.conn.clone();
        let value: Option<u64> = conn
            .get(key)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}")))?;
        Ok(value)
    }

    // ──────────────────────────────────────────────
    // 登录失败计数（登录爆破防护）
    // ──────────────────────────────────────────────

    /// 登录失败计数 Key 前缀
    const LOGIN_FAILURE_PREFIX: &'static str = "login:fail:";

    /// 读取当前失败次数
    pub async fn login_failure_count(&self, scope: &str) -> Result<u64> {
        let key = format!("{}{}", Self::LOGIN_FAILURE_PREFIX, scope);
        let mut conn = self.conn.clone();
        let value: Option<u64> = conn
            .get(key)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}")))?;
        Ok(value.unwrap_or(0))
    }

    /// 记录一次失败并返回累计次数（首次写入时设置窗口过期时间）
    pub async fn record_login_failure(&self, scope: &str, window_seconds: u64) -> Result<u64> {
        let key = format!("{}{}", Self::LOGIN_FAILURE_PREFIX, scope);
        let mut conn = self.conn.clone();
        let count: u64 = conn
            .incr(&key, 1)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}")))?;
        if count == 1 {
            let _: () = conn
                .expire(&key, window_seconds.max(1) as i64)
                .await
                .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}")))?;
        }
        Ok(count)
    }

    /// 登录成功后清除失败计数
    pub async fn clear_login_failures(&self, scope: &str) -> Result<()> {
        let key = format!("{}{}", Self::LOGIN_FAILURE_PREFIX, scope);
        let mut conn = self.conn.clone();
        let _: () = conn
            .del(key)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 删除失败: {e}")))?;
        Ok(())
    }

    // ──────────────────────────────────────────────
    // 限流计数（固定窗口 INCR + EXPIRE）
    // ──────────────────────────────────────────────

    /// 限流 Key 前缀
    const RATE_LIMIT_PREFIX: &'static str = "ratelimit:";

    /// 检查是否超过限流阈值
    ///
    /// 原子操作：INCR → 设置 EXPIRE（仅首次）→ 返回计数和限额。
    pub async fn check_rate_limit(
        &self,
        key_suffix: &str,
        max_requests: u64,
        window_seconds: u64,
    ) -> Result<RateLimitResult> {
        let key = format!("{}{}", Self::RATE_LIMIT_PREFIX, key_suffix);
        let mut conn = self.conn.clone();

        // 原子递增，返回递增后的值
        let current: u64 = conn.incr(&key, 1).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis INCR 失败: {e}")))?;

        // 首次访问时设置过期时间
        if current == 1 {
            let _: () = conn.expire(&key, window_seconds as i64).await
                .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis EXPIRE 失败: {e}")))?;
        }

        let remaining = if current >= max_requests {
            0
        } else {
            max_requests - current
        };

        Ok(RateLimitResult {
            allowed: current <= max_requests,
            current,
            limit: max_requests,
            remaining,
        })
    }

    /// 检查 IP 级限流
    pub async fn check_ip_rate_limit(
        &self,
        ip: &str,
        max_requests: u64,
        window_seconds: u64,
    ) -> Result<RateLimitResult> {
        self.check_rate_limit(&format!("ip:{}", ip), max_requests, window_seconds)
            .await
    }

    // ──────────────────────────────────────────────
    // 通用键值对（字典缓存等）
    // ──────────────────────────────────────────────

    /// 取字符串值
    pub async fn get_string(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.conn.clone();
        conn.get(key).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis GET失败: {e}")))
    }

    /// 设置字符串值（含过期时间）
    pub async fn set_string(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: () = conn.set_ex(key, value, ttl_seconds).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis SET失败: {e}")))?;
        Ok(())
    }

    // ──────────────────────────────────────────────
    // 高级缓存策略：热点数据 + 自动刷新 + 击穿防护
    // ──────────────────────────────────────────────

    /// 缓存 TTL（秒）
    const CACHE_SHORT_TTL: u64 = 60;      // 1 分钟
    const CACHE_MEDIUM_TTL: u64 = 300;    // 5 分钟
    const CACHE_LONG_TTL: u64 = 3600;     // 1 小时
    const CACHE_REFRESH_AHEAD: u64 = 60;  // 提前刷新时间（秒）

    /// 获取缓存，支持自动刷新
    ///
    /// 当缓存即将过期（剩余 TTL < CACHE_REFRESH_AHEAD）时，
    /// 返回旧值的同时异步刷新缓存，避免缓存雪崩。
    pub async fn get_cache_with_auto_refresh<F, Fut>(
        &self,
        key: &str,
        ttl: u64,
        fetch_fn: F,
    ) -> Result<String>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<String>> + Send,
    {
        let mut conn = self.conn.clone();

        // 1. 先尝试读缓存
        let cached: Option<String> = conn.get(key).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis GET 失败: {e}")))?;

        if let Some(ref value) = cached {
            // 2. 检查剩余 TTL，如果接近过期则异步刷新
            let ttl_remaining: Option<i64> = conn.ttl(key).await.ok();
            if let Some(remaining) = ttl_remaining {
                if remaining > 0 && remaining as u64 <= Self::CACHE_REFRESH_AHEAD {
                    // 异步刷新缓存，不阻塞当前响应
                    let key_owned = key.to_string();
                    let redis_clone = self.clone();
                    tokio::spawn(async move {
                        match fetch_fn().await {
                            Ok(new_value) => {
                                let _ = redis_clone.set_string(&key_owned, &new_value, ttl).await;
                                tracing::debug!("缓存自动刷新: {}", key_owned);
                            }
                            Err(e) => {
                                tracing::warn!("缓存自动刷新失败: {}: {}", key_owned, e);
                            }
                        }
                    });
                }
            }
            return Ok(value.clone());
        }

        // 3. 缓存穿透：使用 SETNX 实现互斥锁防止击穿
        let lock_key = format!("{}:lock", key);
        let lock_acquired: bool = conn.set_nx(&lock_key, "1").await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis SETNX 失败: {e}")))?;

        if lock_acquired {
            // 当前线程获得锁，执行回源查询
            conn.expire::<_, ()>(&lock_key, 10).await.ok(); // 锁 10 秒自动释放
            let value = fetch_fn().await?;
            self.set_string(key, &value, ttl).await?;
            conn.del::<_, ()>(&lock_key).await.ok(); // 释放锁
            Ok(value)
        } else {
            // 其他线程等待锁释放后重试
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            // 重试读缓存
            let retry: Option<String> = conn.get(key).await
                .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 重试失败: {e}")))?;
            Ok(retry.unwrap_or_default())
        }
    }

    /// 批量预热缓存
    pub async fn warmup_cache<K, V>(
        &self,
        entries: Vec<(K, V)>,
        ttl: u64,
    ) where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in &entries {
            let _ = self.set_string(key.as_ref(), value.as_ref(), ttl).await;
        }
        tracing::info!("缓存预热完成: {} 条", entries.len());
    }

    /// 检查用户级限流
    pub async fn check_user_rate_limit(
        &self,
        user_id: &str,
        max_requests: u64,
        window_seconds: u64,
    ) -> Result<RateLimitResult> {
        self.check_rate_limit(&format!("user:{}", user_id), max_requests, window_seconds)
            .await
    }
}
