//! 操作日志中间件
//!
//! 把已认证请求的操作信息异步写入 `audit_logs`。
//!
//! 设计取舍：
//! - 只记录方法、路径与查询串，**不记录请求体**。登录口令、令牌等敏感字段
//!   一旦入库就是长期泄露面；同时读取请求体会破坏下游 extractor。
//! - 该中间件注册在认证中间件之内，因此能拿到已认证用户；
//!   未认证请求不进入受保护路由，也就不会产生审计记录。

use std::time::Instant;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::client_ip;
use crate::router::AppState;

/// 查询串入库前的最大长度
const MAX_QUERY_LEN: usize = 500;

/// 操作日志中间件
pub async fn audit_log_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let params = req.uri().query().map(|q| truncate(q, MAX_QUERY_LEN));

    let auth_user = req.extensions().get::<AuthenticatedUser>().cloned();
    let client_ip = req
        .extensions()
        .get::<client_ip::ClientIp>()
        .map(|c| c.0.clone())
        .unwrap_or_else(|| client_ip::resolve_client_ip(&req, false));

    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as i32;
    let status_code = response.status().as_u16() as i32;

    let pool = state.auth_service.user_repo.pool().clone();

    // 异步写入，不阻塞响应
    tokio::spawn(async move {
        let (user_id, username) = match auth_user {
            Some(u) => (Some(u.user_id), Some(u.username)),
            None => (None, None),
        };
        let action = format!("{method} {path}");

        let result = sqlx::query(
            r#"
            INSERT INTO audit_logs
                (user_id, username, action, method, path, params, status_code, client_ip, duration_ms)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(user_id)
        .bind(username)
        .bind(&action)
        .bind(&method)
        .bind(&path)
        .bind(params)
        .bind(status_code)
        .bind(&client_ip)
        .bind(duration_ms)
        .execute(&pool)
        .await;

        if let Err(e) = result {
            tracing::warn!("写入操作日志失败: {e}");
        }
    });

    Ok(response)
}

/// 截断过长字符串，避免超长查询串撑大日志表
fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    value.chars().take(max_len).collect()
}
