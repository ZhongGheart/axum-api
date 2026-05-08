//! 应用配置管理模块
//!
//! 通过 dotenvy 从 .env 文件加载配置，提供统一的配置访问接口。
//! 所有配置项都有默认值，确保在缺少部分环境变量时仍可运行。

use std::env;
use std::net::SocketAddr;

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
}

impl Config {
    /// 从环境变量加载配置
    ///
    /// # Panics
    ///
    /// 缺少必需的配置项（`DATABASE_URL`, `JWT_SECRET`）时会 panic。
    pub fn from_env() -> Self {
        // 加载 .env 文件（如果存在）
        dotenvy::dotenv().ok();

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
            .unwrap_or_else(|_| "604800".to_string()) // 默认7天
            .parse()
            .expect("JWT_EXPIRATION_SECONDS 必须是有效的数字");

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        Self {
            server_addr,
            database_url,
            jwt_secret,
            jwt_expiration_seconds,
            cors_allowed_origins,
        }
    }
}
