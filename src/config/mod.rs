//! 应用配置管理模块
//!
//! 提供统一的配置访问接口。
//! `dotenvy::dotenv()` 应在 `main()` 中调用，不在本模块内调用。

use std::env;
use std::net::SocketAddr;
use crate::utils::crypto::CryptoConfig;

/// 读写分离数据库配置
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub read_url: Option<String>,
    pub write_url: String,
    pub max_size: u32,
    pub connect_timeout_seconds: u64,
    pub read_max_size: Option<u32>,
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

/// 应用全局配置
#[derive(Debug, Clone)]
pub struct Config {
    /// 服务器监听地址
    pub server_addr: SocketAddr,
    /// 数据库连接字符串
    pub database_url: String,
    /// JWT 密钥
    pub jwt_secret: String,
    /// JWT 过期时间（秒）
    pub jwt_expiration_seconds: u64,
    /// CORS 允许的来源
    pub cors_allowed_origins: Vec<String>,
    /// Redis 配置
    pub redis: RedisConfig,
    /// 数据库连接池配置
    pub pool: PoolConfig,
    /// 限流配置
    pub rate_limit: RateLimitConfig,
    /// 应用密钥
    pub app_secret: String,
    /// 加密配置
    pub crypto: CryptoConfig,
    /// 数据库读写分离配置
    pub database: DatabaseConfig,
    /// 验证码配置
    pub captcha_enabled: bool,
}

impl Config {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let host = env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port: u16 = env::var("SERVER_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse()
            .expect("SERVER_PORT 必须是有效的端口号");

        let server_addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .expect("无法解析 SERVER_HOST 和 SERVER_PORT 的组合");

        let database_url = env::var("DATABASE_URL")
            .expect("缺少 DATABASE_URL 环境变量");

        let jwt_secret = env::var("JWT_SECRET")
            .expect("缺少 JWT_SECRET 环境变量");

        let jwt_expiration_seconds: u64 = env::var("JWT_EXPIRATION_SECONDS")
            .unwrap_or_else(|_| "604800".to_string())
            .parse()
            .expect("JWT_EXPIRATION_SECONDS 必须是有效的数字");

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        // Redis 配置
        let redis = RedisConfig {
            url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
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

        // 应用密钥（用于验证码生成等）
        let app_secret = env::var("APP_SECRET")
            .unwrap_or_else(|_| jwt_secret.clone());

        // 加密配置
        let crypto = CryptoConfig {
            private_key_pem: env::var("RSA_PRIVATE_KEY").unwrap_or_default(),
            public_key_pem: env::var("RSA_PUBLIC_KEY").ok(),
            enabled: env::var("CRYPTO_ENABLED").unwrap_or_else(|_| "true".to_string()) == "true",
            enforced_paths: env::var("CRYPTO_ENFORCED_PATHS")
                .unwrap_or_else(|_| "/api/auth/login,/api/auth/register".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
        };

        // 数据库读写分离
        let database = DatabaseConfig {
            read_url: env::var("DATABASE_READ_URL").ok(),
            write_url: database_url.clone(),
            max_size: pool.max_size,
            connect_timeout_seconds: pool.connect_timeout_seconds,
            read_max_size: Some(
                env::var("DB_READ_POOL_MAX_SIZE")
                    .unwrap_or_else(|_| "30".to_string())
                    .parse()
                    .expect("DB_READ_POOL_MAX_SIZE 必须是有效数字"),
            ),
        };

        // 验证码
        let captcha_enabled = env::var("CAPTCHA_ENABLED")
            .unwrap_or_else(|_| "false".to_string()) == "true";

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

        Self {
            server_addr,
            database_url,
            jwt_secret,
            jwt_expiration_seconds,
            cors_allowed_origins,
            redis,
            pool,
            rate_limit,
            app_secret,
            crypto,
            database,
            captcha_enabled,
        }
    }
}
