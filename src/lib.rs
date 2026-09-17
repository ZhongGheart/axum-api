//! Axum Admin 后端库入口
//!
//! HTTP 二进制入口在 `src/main.rs`。这里把各层模块暴露为库目标，
//! 使 `tests/` 下的集成测试可以直接构建路由并用 `oneshot` 驱动真实请求。

#![recursion_limit = "256"]

pub mod config;
pub mod controller;
pub mod docs;
pub mod error;
pub mod middleware;
pub mod model;
pub mod repository;
pub mod router;
pub mod service;
pub mod utils;
