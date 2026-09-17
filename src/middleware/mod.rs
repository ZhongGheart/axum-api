//! 中间件层模块导出
//!
//! axum 中间件的错误类型固定为 `Response`（内部结构较大属框架约定），
//! `clippy::result_large_err` 在本层属结构性误报，故仅在本模块范围内关闭。
#![allow(clippy::result_large_err)]

pub mod api_metrics;
pub mod audit_log;
pub mod auth;
pub mod client_ip;
pub mod rate_limit;
pub mod request_id;
