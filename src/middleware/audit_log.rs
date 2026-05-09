//! 操作日志中间件
//!
//! 记录用户操作、请求参数、响应结果、IP、时间。
//! 可通过配置开关控制是否启用。

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::middleware::auth::AuthenticatedUser;
use crate::router::AppState;

/// 操作日志配置
#[derive(Debug, Clone)]
pub struct AuditLogConfig {
    /// 是否启用操作日志
    pub enabled: bool,
}

/// 操作日志状态（全局单例）
pub static AUDIT_LOG_ENABLED: AtomicBool = AtomicBool::new(true);

/// 设置操作日志开关
pub fn set_audit_log_enabled(enabled: bool) {
    AUDIT_LOG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// 操作日志中间件
///
/// 记录每个请求的操作信息到 `audit_logs` 表。
/// 不侵入原有业务逻辑，仅在请求完成后异步写入。
pub async fn audit_log_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    if !AUDIT_LOG_ENABLED.load(Ordering::Relaxed) {
        return Ok(next.run(req).await);
    }

    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let client_ip = req
        .headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            req.headers()
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or("unknown")
        .to_string();

    // 提取用户信息（如果已认证）
    let user_id = req.extensions().get::<AuthenticatedUser>().map(|u| u.user_id);
    let username = req.extensions().get::<AuthenticatedUser>().map(|u| u.role.clone());

    // 提取请求参数
    let params = req.headers().get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .map(|s| format!("Content-Type: {}", s));

    // 执行请求
    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as i32;
    let status_code = response.status().as_u16() as i32;

    // 异步写入日志（不阻塞响应）
    let pool = state.auth_service.user_repo.pool.clone();
    let log_user_id = user_id;
    let log_username = username;
    let log_method = method;
    let log_path = path;
    let log_params = params;
    let log_ip = client_ip;

    tokio::spawn(async move {
        let action = format!("{} {}", log_method, log_path);
        let _ = sqlx::query(
            r#"
            INSERT INTO audit_logs (user_id, username, action, method, path, params, status_code, client_ip, duration_ms)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(log_user_id)
        .bind(log_username)
        .bind(&action)
        .bind(&log_method)
        .bind(&log_path)
        .bind(log_params)
        .bind(status_code)
        .bind(&log_ip)
        .bind(duration_ms)
        .execute(&pool)
        .await;
    });

    Ok(response)
}
