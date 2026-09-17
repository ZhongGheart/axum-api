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
    /// 登录失败锁定阈值（账号与 IP 各自独立计数）
    pub login_max_failures: u64,
    /// 登录失败计数窗口（秒）
    pub login_failure_window_seconds: u64,
}

/// 应用全局配置
#[derive(Debug, Clone)]
pub struct Config {
    /// 服务器监听地址
    pub server_addr: SocketAddr,
    /// JWT 密钥
    pub jwt_secret: String,
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
    /// 启动时是否自动执行数据库迁移
    pub migrate_on_startup: bool,
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

        Self {
            server_addr,
            jwt_secret,
            jwt_expiration_seconds,
            cors_allowed_origins,
            redis,
            rate_limit,
            security,
            database,
            migrate_on_startup,
        }
    }
}
