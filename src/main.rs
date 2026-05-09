//! 应用入口
//!
//! 初始化 Tracing 日志系统、加载配置、构建路由、启动 HTTP 服务器。

#![recursion_limit = "256"]

mod config;
mod controller;
mod docs;
mod error;
mod middleware;
mod model;
mod repository;
mod router;
mod service;
mod utils;

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::router::create_router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── 初始化 Tracing 日志系统 ─────────────────────────────────
    init_tracing();

    // ── 从 .env 加载环境变量（若存在） ─────────────────────────
    dotenvy::dotenv().ok();

    // ── 加载配置 ───────────────────────────────────────────────
    let config = config::Config::from_env();
    let addr = config.server_addr;
    tracing::info!("配置加载完成，监听地址: {addr}");

    // ── 构建路由 ───────────────────────────────────────────────
    let app = create_router(config).await?;

    // ── 启动服务器（带优雅关闭） ────────────────────────────────
    tracing::info!("服务器启动中 → http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// 监听 SIGTERM / SIGINT 信号，触发优雅关闭
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("无法注册 Ctrl+C 信号处理器");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("无法注册 SIGTERM 信号处理器")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl+C，开始优雅关闭..."),
        _ = terminate => tracing::info!("收到 SIGTERM，开始优雅关闭..."),
    }
}

/// 初始化 Tracing 日志系统
///
/// 配置说明：
/// - 支持通过 `RUST_LOG` 环境变量控制日志级别（如 `RUST_LOG=debug`）
/// - 默认级别为 `info`
/// - 同时输出到终端（格式化）和可用于日志收集的结构化输出
/// - 包含 span 追踪信息，便于请求链路定位
fn init_tracing() {
    // 环境变量过滤：RUST_LOG=debug ./target/release/axum-api
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // 格式化终端输出（带颜色、时间戳、目标模块）
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();
}
