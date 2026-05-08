//! Redis 通用工具模块
//!
//! 封装异步 Redis 客户端，提供连接管理、Token 黑名单、限流计数等通用能力。

use redis::aio::ConnectionManager;
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

impl RedisClient {
    /// 从配置创建 Redis 客户端连接
    pub async fn new(config: &RedisConfig) -> Result<Self> {
        let client = redis::Client::open(config.url.as_str())
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 连接失败: {e}")))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 连接管理初始化失败: {e}")))?;

        Ok(Self { conn })
    }

    /// 从已存在的客户端构建（用于共享同一客户端）
    #[allow(dead_code)]
    pub async fn from_client(client: redis::Client) -> Result<Self> {
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 连接管理初始化失败: {e}")))?;
        Ok(Self { conn })
    }

    // ──────────────────────────────────────────────
    // Token 黑名单
    // ──────────────────────────────────────────────

    /// Token 黑名单 Key 前缀
    const TOKEN_BLACKLIST_PREFIX: &'static str = "token:blacklist:";

    /// 将 Token 加入黑名单，TTL 由 Token 剩余有效期决定
    pub async fn add_token_to_blacklist(
        &self,
        token_sub: &str,
        token_exp: u64,
    ) -> Result<()> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, token_sub);
        let now = chrono::Utc::now().timestamp() as u64;
        // 剩余有效秒数 = 令牌过期时间 - 当前时间，最少 1 秒
        let ttl = if token_exp > now {
            token_exp - now
        } else {
            1
        };

        let mut conn = self.conn.clone();
        let _: () = conn.set_ex(key, "1", ttl).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}")))?;

        Ok(())
    }

    /// 从黑名单中移除（登录成功时调用，清除旧登出记录）
    pub async fn remove_token_blacklist(&self, token_sub: &str) -> Result<()> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, token_sub);
        let mut conn = self.conn.clone();
        let _: () = conn.del(key).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 删除失败: {e}")))?;
        Ok(())
    }

    /// 检查 Token 是否在黑名单中
    pub async fn is_token_blacklisted(&self, token_sub: &str) -> Result<bool> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, token_sub);
        let mut conn = self.conn.clone();
        let exists: Option<String> = conn.get(key).await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}")))?;
        Ok(exists.is_some())
    }

    // ──────────────────────────────────────────────
    // 限流计数（滑动窗口 INCR + EXPIRE）
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
