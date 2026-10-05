//! 应用配置管理模块
//!
//! 提供统一的配置访问接口。
//! `dotenvy::dotenv()` 应在 `main()` 中调用，不在本模块内调用。

use std::env;
use std::net::SocketAddr;

/// 已知示例占位密钥，生产启动时必须拒绝
const PLACEHOLDER_JWT_SECRETS: [&str; 2] = [
    "your-super-secret-jwt-key-change-in-production",
    "change-me",
];

/// 数据库连接配置
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub write_url: String,
    pub max_size: u32,
    pub connect_timeout_seconds: u64,
}

/// Redis 配置
#[derive(Debug, Clone)]
pub struct RedisConfig {
    /// Redis 连接字符串
    pub url: String,
}

/// 数据库连接池配置
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 最大连接数
    pub max_size: u32,
    /// 连接超时（秒）
    pub connect_timeout_seconds: u64,
}

/// 限流配置
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 单个 IP 每分钟最大请求数
    pub ip_max_requests: u64,
    /// IP 限流时间窗口（秒）
    pub ip_window_seconds: u64,
    /// 单个用户每分钟最大请求数
    pub user_max_requests: u64,
    /// 用户限流时间窗口（秒）
    pub user_window_seconds: u64,
}

/// 安全策略配置
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// 是否信任上游代理的转发头（决定客户端 IP 取值方式）
    pub trust_proxy_headers: bool,
    /// 登录失败锁定阈值（账号与 IP 各自独立计数）——**回落值**
    ///
    /// v0.22.0 起正常路径读 `system_settings` 表里的
    /// `security.login.max_failures`，这里只在参数表读不到时兜底。
    /// 改它仍然有效，但要重启进程；要在运行时生效请用「系统参数」页。
    pub login_max_failures: u64,
    /// 登录失败计数窗口（秒）——**回落值**，同上
    pub login_failure_window_seconds: u64,
}

/// 审计日志保留策略
#[derive(Debug, Clone)]
pub struct AuditLogConfig {
    /// 保留天数；`0` 表示关闭自动清理（由运维手工处理）
    pub retention_days: u32,
    /// 清理任务运行间隔（秒）
    pub cleanup_interval_seconds: u64,
    /// 单批删除行数上限：把长事务切碎，避免长时间持锁与 WAL 膨胀
    pub cleanup_batch_size: i64,
    /// 单轮清理最多执行多少批；删空即提前结束
    pub cleanup_max_batches: u32,
}

/// 接口耗时指标聚合配置
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// 本地增量缓冲的刷新间隔（秒）
    ///
    /// 决定"进程崩溃最多丢多少指标"的上界：间隔内的增量还在本地缓冲里。
    pub flush_interval_seconds: u64,
    /// Redis 指标键的存活时间（秒），兜底防止长期不再访问的键堆积
    pub key_ttl_seconds: u64,
    /// 本地缓冲允许的最大端点数：Redis 长期不可用时防止无限增长
    pub max_buffered_endpoints: usize,
}

/// 存储后端选择
///
/// 由 `STORAGE_BACKEND` 决定，**默认 `local`**——
/// 未设置任何新环境变量时行为与 v0.26.0 逐字一致，不给存量部署增加任何前置条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    /// 本地磁盘（`UPLOAD_DIR`）
    Local,
    /// S3 兼容对象存储（S3 / MinIO / OSS / COS 等）
    S3,
}

/// S3 兼容后端配置
///
/// 只在 `STORAGE_BACKEND=s3` 时读取。
#[derive(Debug, Clone)]
pub struct S3Config {
    /// endpoint。AWS 官方可留空走 `https://s3.<region>.amazonaws.com`；
    /// MinIO / OSS / COS 这类自建或第三方必须显式给。
    pub endpoint: Option<String>,
    /// bucket 名
    pub bucket: String,
    /// 区域
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// 对外访问用的基地址，如 CDN 域名 `https://cdn.example.com`
    ///
    /// **要包含 bucket 段**（除非前面挂了 CDN 抹掉了 bucket 这一层）：
    /// 直连 S3 时应是 `{endpoint}/{bucket}`，挂 CDN 时是 CDN 根域名。
    ///
    /// 留空则按 `{endpoint}/{bucket}` 推导（AWS 则推导到
    /// `https://{bucket}.s3.{region}.amazonaws.com`）——
    /// 那只在 bucket 公开读时成立。私有 bucket 必须显式给 CDN
    /// 或签名网关地址，否则头像存得进去、显示不出来。
    pub public_base_url: Option<String>,
    /// bucket 内的 key 前缀（默认空）
    ///
    /// key 本身已经以 `avatars/` 开头，所以默认不再加前缀——
    /// 默认值若也是 `avatars`，URL 会变成 `avatars/avatars/x.png`，
    /// 看着像笔误而实际能工作，排查时很费时间。
    ///
    /// 一个 bucket 常与别的应用共用时，用它隔开
    /// （如 `axum-api/prod`），别人的清理脚本就不会把头像一起删掉。
    pub key_prefix: String,
}

/// 头像存储配置（v0.27.0 由 `UploadConfig` 改名而来）
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// 后端选择
    pub backend: StorageBackend,
    /// 本地落盘根目录（仅 `backend = local` 时使用，相对路径按进程工作目录解析）
    ///
    /// 容器里必须挂卷到此处，否则重启即丢图。见 docker-compose.yml 的 volume 配置。
    pub dir: String,
    /// 单文件字节上限
    ///
    /// 超限直接 413。这里**不用** multipart 流式落盘再判大小：
    /// 先落盘再删会在磁盘上留下攻击者任意大小的垃圾。
    pub max_file_size: usize,
    /// 允许的图片 MIME 白名单
    pub allowed_mime_types: Vec<String>,
    /// S3 参数（仅 `backend = s3` 时有意义）
    pub s3: Option<S3Config>,
}

/// 应用全局配置
#[derive(Debug, Clone)]
pub struct Config {
    /// 服务器监听地址
    pub server_addr: SocketAddr,
    /// JWT 密钥
    pub jwt_secret: String,
    /// 两步验证密钥的加密密钥（AES-256-GCM）
    ///
    /// **不设为必填是有意的**：v0.25.0 之前的所有部署都没有这个变量，
    /// 若改成必填，一次发版就会让所有存量实例起不来——而"多一个部署前置条件"
    /// 是需要单独决策的事（与 v0.24.0 未排期的邮件通道同一性质）。
    /// 未设置时从 `JWT_SECRET` 派生（带独立前缀），并在启动日志里告警。
    pub totp_encryption_key: String,
    /// JWT 过期时间（秒）
    pub jwt_expiration_seconds: u64,
    /// CORS 允许的来源
    pub cors_allowed_origins: Vec<String>,
    /// Redis 配置
    pub redis: RedisConfig,
    /// 限流配置
    pub rate_limit: RateLimitConfig,
    /// 安全策略配置
    pub security: SecurityConfig,
    /// 数据库连接配置
    pub database: DatabaseConfig,
    /// 审计日志保留策略
    pub audit_log: AuditLogConfig,
    /// 接口耗时指标聚合配置
    pub metrics: MetricsConfig,
    /// 头像上传配置
    pub storage: StorageConfig,
    /// 启动时是否自动执行数据库迁移
    pub migrate_on_startup: bool,
}

/// 读 S3 参数，只在 `STORAGE_BACKEND=s3` 时校验
///
/// **启动期就 panic，而不是等到第一次上传头像**：
/// 配错 endpoint/bucket 的故障如果拖到运行时才发现，表现是"用户传头像报 500"，
/// 而日志里只有一行 opendal 的英文错误，没人知道是环境变量少写了。
fn read_s3_config() -> Option<S3Config> {
    let backend = env::var("STORAGE_BACKEND")
        .unwrap_or_else(|_| "local".to_string())
        .to_lowercase();
    if backend != "s3" {
        return None;
    }
    let required = |name: &str| -> String {
        env::var(name).unwrap_or_else(|_| {
            panic!("STORAGE_BACKEND=s3 时必须设置 {name}（S3 兼容后端的连接参数）")
        })
    };
    let bucket = required("S3_BUCKET");
    let region = env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let access_key_id = required("S3_ACCESS_KEY_ID");
    let secret_access_key = required("S3_SECRET_ACCESS_KEY");
    // AWS 官方可以不给 endpoint（由 region 推导），MinIO / OSS / COS 必须给
    let endpoint = env::var("S3_ENDPOINT")
        .ok()
        .map(|v| v.trim_end_matches('/').to_string());
    if endpoint.is_none() {
        tracing::warn!(
            "S3_ENDPOINT 未设置，将按 AWS 默认端点推导；用 MinIO / OSS / COS 时必须显式设置"
        );
    }
    Some(S3Config {
        endpoint,
        bucket,
        region,
        access_key_id,
        secret_access_key,
        public_base_url: env::var("S3_PUBLIC_BASE_URL")
            .ok()
            .map(|v| v.trim_end_matches('/').to_string()),
        key_prefix: env::var("S3_KEY_PREFIX")
            .unwrap_or_default()
            .trim_matches('/')
            .to_string(),
    })
}

impl Config {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

        let host = env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("SERVER_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse()
            .expect("SERVER_PORT 必须是有效的端口号");

        let server_addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .expect("无法解析 SERVER_HOST 和 SERVER_PORT 的组合");

        let database_url = env::var("DATABASE_URL").expect("缺少 DATABASE_URL 环境变量");

        let jwt_secret = env::var("JWT_SECRET").expect("缺少 JWT_SECRET 环境变量");
        // 拒绝弱密钥与示例占位值，避免签名可被伪造
        if jwt_secret.len() < 32 {
            panic!("JWT_SECRET 长度必须不少于 32 个字符（可用 `openssl rand -base64 48` 生成）");
        }
        if PLACEHOLDER_JWT_SECRETS.contains(&jwt_secret.as_str()) {
            if app_env == "production" {
                panic!("生产环境禁止使用示例占位 JWT_SECRET，请用 `openssl rand -base64 48` 生成");
            }
            tracing::warn!("JWT_SECRET 使用示例占位值，仅可用于本地开发");
        }

        let jwt_expiration_seconds: u64 = env::var("JWT_EXPIRATION_SECONDS")
            .unwrap_or_else(|_| "604800".to_string())
            .parse()
            .expect("JWT_EXPIRATION_SECONDS 必须是有效的数字");

        // ── 两步验证密钥的加密密钥 ──────────────────────────────
        //
        // 未设置时从 JWT_SECRET 派生，**并加独立前缀**：直接复用 JWT_SECRET
        // 原文会让"轮换 JWT 密钥"这个运维动作静默地让所有已绑定用户的
        // TOTP 密钥永久不可解（本仓没有邮件通道，他们无法自助重绑）。
        // 加前缀派生至少把两把密钥在域上分开，也让派生过程可审计。
        let (totp_encryption_key, totp_key_derived) = match env::var("TOTP_ENCRYPTION_KEY") {
            Ok(key) => (key, false),
            Err(_) => (format!("derived:{jwt_secret}"), true),
        };
        if totp_key_derived {
            tracing::warn!(
                "TOTP_ENCRYPTION_KEY 未设置，两步验证密钥的加密密钥已从 JWT_SECRET 派生；\
                 建议显式配置独立密钥，否则轮换 JWT_SECRET 会使已绑定用户的 2FA 失效"
            );
        }

        let cors_allowed_origins: Vec<String> = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3000".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        if app_env == "production" && cors_allowed_origins.iter().any(|o| o == "*") {
            panic!("生产环境禁止 CORS_ALLOWED_ORIGINS=*，请显式列出允许的来源");
        }

        // Redis 配置
        let redis = RedisConfig {
            url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
        };

        // 数据库连接池配置
        let pool = PoolConfig {
            max_size: env::var("DB_POOL_MAX_SIZE")
                .unwrap_or_else(|_| "20".to_string())
                .parse()
                .expect("DB_POOL_MAX_SIZE 必须是有效的数字"),
            connect_timeout_seconds: env::var("DB_CONNECT_TIMEOUT")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .expect("DB_CONNECT_TIMEOUT 必须是有效的数字"),
        };

        let database = DatabaseConfig {
            write_url: database_url,
            max_size: pool.max_size,
            connect_timeout_seconds: pool.connect_timeout_seconds,
        };

        // 启动时自动执行数据库迁移（关闭后需由独立迁移步骤保证表结构）
        let migrate_on_startup =
            env::var("MIGRATE_ON_STARTUP").unwrap_or_else(|_| "true".to_string()) != "false";

        // 审计日志保留策略
        let audit_log = AuditLogConfig {
            retention_days: env::var("AUDIT_LOG_RETENTION_DAYS")
                .unwrap_or_else(|_| "90".to_string())
                .parse()
                .expect("AUDIT_LOG_RETENTION_DAYS 必须是有效的数字（0 表示关闭自动清理）"),
            cleanup_interval_seconds: env::var("AUDIT_LOG_CLEANUP_INTERVAL_SECONDS")
                .unwrap_or_else(|_| "3600".to_string())
                .parse()
                .expect("AUDIT_LOG_CLEANUP_INTERVAL_SECONDS 必须是有效的数字"),
            cleanup_batch_size: env::var("AUDIT_LOG_CLEANUP_BATCH_SIZE")
                .unwrap_or_else(|_| "10000".to_string())
                .parse()
                .expect("AUDIT_LOG_CLEANUP_BATCH_SIZE 必须是有效的数字"),
            cleanup_max_batches: env::var("AUDIT_LOG_CLEANUP_MAX_BATCHES")
                .unwrap_or_else(|_| "20".to_string())
                .parse()
                .expect("AUDIT_LOG_CLEANUP_MAX_BATCHES 必须是有效的数字"),
        };

        // 间隔为 0 会让定时任务空转刷 Redis，这里兜底成 1 秒
        let metrics_flush_interval_seconds = env::var("METRICS_FLUSH_INTERVAL_SECONDS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u64>()
            .expect("METRICS_FLUSH_INTERVAL_SECONDS 必须是有效的数字")
            .max(1);

        let metrics = MetricsConfig {
            flush_interval_seconds: metrics_flush_interval_seconds,
            key_ttl_seconds: env::var("METRICS_KEY_TTL_SECONDS")
                .unwrap_or_else(|_| "604800".to_string())
                .parse()
                .expect("METRICS_KEY_TTL_SECONDS 必须是有效的数字"),
            max_buffered_endpoints: env::var("METRICS_MAX_BUFFERED_ENDPOINTS")
                .unwrap_or_else(|_| "10000".to_string())
                .parse()
                .expect("METRICS_MAX_BUFFERED_ENDPOINTS 必须是有效的数字"),
        };

        // 限流配置
        let rate_limit = RateLimitConfig {
            ip_max_requests: env::var("RATE_LIMIT_IP_MAX")
                .unwrap_or_else(|_| "100".to_string())
                .parse()
                .expect("RATE_LIMIT_IP_MAX 必须是有效的数字"),
            ip_window_seconds: env::var("RATE_LIMIT_IP_WINDOW")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .expect("RATE_LIMIT_IP_WINDOW 必须是有效的数字"),
            user_max_requests: env::var("RATE_LIMIT_USER_MAX")
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .expect("RATE_LIMIT_USER_MAX 必须是有效的数字"),
            user_window_seconds: env::var("RATE_LIMIT_USER_WINDOW")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .expect("RATE_LIMIT_USER_WINDOW 必须是有效的数字"),
        };

        let security = SecurityConfig {
            trust_proxy_headers: env::var("TRUST_PROXY_HEADERS")
                .unwrap_or_else(|_| "false".to_string())
                == "true",
            login_max_failures: env::var("LOGIN_MAX_FAILURES")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .expect("LOGIN_MAX_FAILURES 必须是有效的数字"),
            login_failure_window_seconds: env::var("LOGIN_FAILURE_WINDOW")
                .unwrap_or_else(|_| "300".to_string())
                .parse()
                .expect("LOGIN_FAILURE_WINDOW 必须是有效的数字"),
        };

        let storage = StorageConfig {
            backend: match env::var("STORAGE_BACKEND")
                .unwrap_or_else(|_| "local".to_string())
                .to_lowercase()
                .as_str()
            {
                // 留空视作 local：有人在 compose 里写了 `STORAGE_BACKEND=` 却以为没生效
                "local" | "" => StorageBackend::Local,
                "s3" => StorageBackend::S3,
                other => panic!("STORAGE_BACKEND 只能是 local 或 s3，当前是 {other:?}"),
            },
            dir: env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string()),
            max_file_size: env::var("UPLOAD_MAX_FILE_SIZE")
                .unwrap_or_else(|_| "2097152".to_string())
                .parse()
                .expect("UPLOAD_MAX_FILE_SIZE 必须是有效的字节数"),
            // 白名单而非黑名单：新增格式要显式放行，
            // 否则将来某个奇怪类型被塞进来就是一次存储型攻击面
            allowed_mime_types: ["image/jpeg", "image/png", "image/webp", "image/gif"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            s3: read_s3_config(),
        };

        Self {
            server_addr,
            jwt_secret,
            totp_encryption_key,
            jwt_expiration_seconds,
            cors_allowed_origins,
            redis,
            rate_limit,
            security,
            database,
            audit_log,
            metrics,
            storage,
            migrate_on_startup,
        }
    }
}
