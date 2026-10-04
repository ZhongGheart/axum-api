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

/// 会话元信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionRecord {
    /// 令牌唯一标识（吊销单个会话时用它）
    ///
    /// 必须存进记录里：不存的话，"列出该用户全部会话"的接口就只给出一堆
    /// 无法区分的条目——管理员看得到有几台设备，却没法踢掉其中一台。
    pub jti: String,
    /// 用户 ID
    pub user_id: uuid::Uuid,
    /// 用户名（展示用，不作鉴权依据——鉴权只看令牌本身）
    pub username: String,
    /// 客户端 IP
    pub client_ip: String,
    /// 登录时刻（Unix 毫秒）
    pub login_at_ms: i64,
    /// 令牌到期时刻（Unix 毫秒）
    pub expires_at_ms: i64,
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
        let client = redis::Client::open(config.url.as_str()).map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 连接失败: {e}"))
        })?;

        let conn = ConnectionManager::new_with_config(client, redis_manager_config())
            .await
            .map_err(|e| {
                crate::error::AppError::InternalServerError(format!(
                    "Redis 连接管理初始化失败: {e}"
                ))
            })?;

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
        let _: () = conn.set_ex(key, "1", ttl).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}"))
        })?;

        Ok(())
    }

    /// 查询令牌是否已被注销
    pub async fn is_token_blacklisted(&self, jti: &str) -> Result<bool> {
        let key = format!("{}{}", Self::TOKEN_BLACKLIST_PREFIX, jti);
        let mut conn = self.conn.clone();
        let exists: Option<String> = conn.get(key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}"))
        })?;
        Ok(exists.is_some())
    }

    /// 吊销某用户当前及之前签发的全部令牌
    ///
    /// 记录"吊销时间点"（**毫秒**），签发时间早于该时间点的令牌一律失效。
    /// 新登录签发的令牌不早于该时间点，因此不会被误伤。
    /// TTL 取令牌最长有效期，过期后键自动清理。
    ///
    /// 必须毫秒而非秒：JWT 标准的 `iat` 只有秒级精度，若水位也只存到秒，
    /// "吊销前签发"与"吊销后签发"会落在同一秒内无法区分——
    /// 秒级方案只能二选一（放过旧令牌 / 误伤新登录），两个都是错的。
    /// 与 `Claims::iat_ms` 配套使用。
    pub async fn revoke_user_sessions(
        &self,
        user_id: &uuid::Uuid,
        ttl_seconds: u64,
    ) -> Result<u64> {
        let key = format!("{}{}", Self::USER_REVOKED_PREFIX, user_id);
        let revoked_before = chrono::Utc::now().timestamp_millis() as u64;
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(key, revoked_before, ttl_seconds.max(1))
            .await
            .map_err(|e| {
                crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}"))
            })?;
        Ok(revoked_before)
    }

    /// 读取用户会话吊销时间点；未吊销返回 `None`
    pub async fn user_revoked_before(&self, user_id: &uuid::Uuid) -> Result<Option<u64>> {
        let key = format!("{}{}", Self::USER_REVOKED_PREFIX, user_id);
        let mut conn = self.conn.clone();
        let value: Option<u64> = conn.get(key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}"))
        })?;
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
        let value: Option<u64> = conn.get(key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 查询失败: {e}"))
        })?;
        Ok(value.unwrap_or(0))
    }

    /// 记录一次失败并返回累计次数（首次写入时设置窗口过期时间）
    pub async fn record_login_failure(&self, scope: &str, window_seconds: u64) -> Result<u64> {
        let key = format!("{}{}", Self::LOGIN_FAILURE_PREFIX, scope);
        let mut conn = self.conn.clone();
        let count: u64 = conn.incr(&key, 1).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}"))
        })?;
        if count == 1 {
            let _: () = conn
                .expire(&key, window_seconds.max(1) as i64)
                .await
                .map_err(|e| {
                    crate::error::AppError::InternalServerError(format!("Redis 写入失败: {e}"))
                })?;
        }
        Ok(count)
    }

    // ──────────────────────────────────────────────
    // 会话登记（谁在线 / 单会话吊销）
    // ──────────────────────────────────────────────

    /// 会话键前缀
    ///
    /// 键形如 `sess:{user_id}:{jti}`——**把 user_id 放进键里**，
    /// 于是"列出某人的全部会话"是一次 `SCAN sess:{user_id}:*`，
    /// 不需要额外维护一份索引集合，也就没有"索引与实际不一致"这种状态。
    ///
    /// 若只用 `sess:{jti}`，列举就得全库 SCAN 再逐条比对 user_id：
    /// 键空间里每个活跃令牌一条记录，那等于把 O(全部会话) 的扫描
    /// 放在管理员随手可点的按钮上。
    const SESSION_PREFIX: &'static str = "sess:";

    /// 登记一个会话
    ///
    /// TTL **必须**等于令牌剩余寿命，这样 Redis 会在令牌失效时自动清掉这条记录。
    /// 少了这个约束，从未登出、永不过期的令牌记录会让键空间单调增长——
    /// 而"谁在线"这个列表没有任何东西会替我们做清理。
    pub async fn register_session(&self, record: &SessionRecord, ttl_seconds: u64) -> Result<()> {
        let key = Self::session_key(record.user_id, &record.jti);
        let payload = serde_json::to_string(record).map_err(|e| {
            crate::error::AppError::InternalServerError(format!("会话序列化失败: {e}"))
        })?;
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(key, payload, ttl_seconds.max(1))
            .await
            .map_err(|e| {
                crate::error::AppError::InternalServerError(format!("Redis 写入会话失败: {e}"))
            })?;
        Ok(())
    }

    /// 列出某用户的全部会话（按登录时间倒序）
    pub async fn list_sessions(&self, user_id: uuid::Uuid) -> Result<Vec<SessionRecord>> {
        let mut conn = self.conn.clone();
        let pattern = format!("{}{}:*", Self::SESSION_PREFIX, user_id);
        let mut cursor: u64 = 0;
        let mut out: Vec<SessionRecord> = Vec::new();
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await
                .map_err(|e| {
                    crate::error::AppError::InternalServerError(format!("Redis SCAN 失败: {e}"))
                })?;
            cursor = next;
            if !keys.is_empty() {
                let vals: Vec<String> = redis::cmd("MGET")
                    .arg(&keys)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| {
                        crate::error::AppError::InternalServerError(format!("Redis 读取失败: {e}"))
                    })?;
                // MGET 在键已过期时返回空串而非 nil 变体，这里统一交给
                // 下面的解析失败分支跳过——一条脏记录不该让整个列表查不出来。
                for v in vals {
                    // 单条解析失败就跳过而不是整体报错：一条脏数据不该
                    // 让管理员连"这个人在哪些设备登录"都看不到。
                    match serde_json::from_str::<SessionRecord>(&v) {
                        Ok(r) => out.push(r),
                        Err(e) => tracing::warn!("会话记录解析失败，已跳过: {e}"),
                    }
                }
            }
            if cursor == 0 {
                break;
            }
        }
        out.sort_by(|a, b| b.login_at_ms.cmp(&a.login_at_ms));
        Ok(out)
    }

    /// 查询单个会话；不存在返回 `None`
    ///
    /// 查不到有两种原因：令牌已过期（键已自动清理），或已被登出/吊销。
    /// 调用方据此拒绝吊销一个不存在的会话，而不是回"成功"——
    /// 那会让界面显示已下线，而令牌其实还在有效期内（键可能因故丢失）。
    pub async fn get_session(
        &self,
        user_id: uuid::Uuid,
        jti: &str,
    ) -> Result<Option<SessionRecord>> {
        let mut conn = self.conn.clone();
        let key = Self::session_key(user_id, jti);
        let val: Option<String> = conn.get(&key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 读取会话失败: {e}"))
        })?;
        match val {
            None => Ok(None),
            // 这里与 `list_sessions` 不同：**不能**跳过解析失败。
            // 列举场景下少一条可以接受（列表本来就只是参考），
            // 但"查某会话是否存在"是要据此决定放不放行的，
            // 解析失败必须当"查不到"处理并让调用方拒绝——绝不能悄悄当成有效。
            Some(s) => match serde_json::from_str::<SessionRecord>(&s) {
                Ok(r) => Ok(Some(r)),
                Err(e) => {
                    tracing::warn!("会话记录解析失败，按不存在处理: {e}");
                    Ok(None)
                }
            },
        }
    }

    /// 删除一条会话登记（登出 / 吊销时调用）
    pub async fn remove_session(&self, user_id: uuid::Uuid, jti: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: usize = conn
            .del(Self::session_key(user_id, jti))
            .await
            .map_err(|e| {
                crate::error::AppError::InternalServerError(format!("Redis 删除会话失败: {e}"))
            })?;
        Ok(())
    }

    /// 会话键
    fn session_key(user_id: uuid::Uuid, jti: &str) -> String {
        format!("{}{}:{}", Self::SESSION_PREFIX, user_id, jti)
    }

    /// 登录成功后清除失败计数
    pub async fn clear_login_failures(&self, scope: &str) -> Result<()> {
        let key = format!("{}{}", Self::LOGIN_FAILURE_PREFIX, scope);
        let mut conn = self.conn.clone();
        let _: () = conn.del(key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis 删除失败: {e}"))
        })?;
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
        let current: u64 = conn.incr(&key, 1).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis INCR 失败: {e}"))
        })?;

        // 首次访问时设置过期时间
        if current == 1 {
            let _: () = conn
                .expire(&key, window_seconds as i64)
                .await
                .map_err(|e| {
                    crate::error::AppError::InternalServerError(format!("Redis EXPIRE 失败: {e}"))
                })?;
        }

        let remaining = max_requests.saturating_sub(current);

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
        conn.get(key)
            .await
            .map_err(|e| crate::error::AppError::InternalServerError(format!("Redis GET失败: {e}")))
    }

    /// 设置字符串值（含过期时间）
    pub async fn set_string(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: () = conn.set_ex(key, value, ttl_seconds).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis SET失败: {e}"))
        })?;
        Ok(())
    }

    /// 删除任意键（用于字典等业务缓存失效）
    pub async fn delete_key(&self, key: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: () = conn.del(key).await.map_err(|e| {
            crate::error::AppError::InternalServerError(format!("Redis DEL 失败: {e}"))
        })?;
        Ok(())
    }

    /// 按前缀删除所有键，返回**实际删除的键数**
    ///
    /// 用 SCAN 增量遍历而不是 KEYS：KEYS 会一次性遍历整个键空间并阻塞 Redis
    /// 的单线程事件循环，而这个操作是管理员在界面上随时能点的。
    /// 游标循环必须每轮都判 `cursor == 0`，否则会漏掉最后一批键。
    pub async fn delete_by_prefix(&self, prefix: &str) -> Result<u64> {
        let mut conn = self.conn.clone();
        let pattern = format!("{prefix}*");
        let mut cursor: u64 = 0;
        let mut deleted: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(500)
                .query_async(&mut conn)
                .await
                .map_err(|e| {
                    crate::error::AppError::InternalServerError(format!("Redis SCAN 失败: {e}"))
                })?;
            cursor = next;
            if !keys.is_empty() {
                let n: u64 = conn.del(&keys).await.map_err(|e| {
                    crate::error::AppError::InternalServerError(format!("Redis DEL 失败: {e}"))
                })?;
                deleted += n;
            }
            if cursor == 0 {
                break;
            }
        }
        Ok(deleted)
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
