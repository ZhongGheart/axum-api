//! 全局异常处理模块
//!
//! 定义 `AppError` 枚举，覆盖所有业务异常场景，
//! 实现 `IntoResponse` trait 以自动转换为统一错误响应。
//! 同时实现全局 panic 捕获中间件。

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// 应用自定义错误类型
///
/// 覆盖认证、验证、数据库、内部等各类异常场景。
/// 所有错误都实现了 `IntoResponse`，自动转换为统一的 JSON 错误响应。
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum AppError {
    /// 错误的请求（参数校验失败等）
    #[error("错误的请求: {0}")]
    BadRequest(String),

    /// 未授权（需登录）
    #[error("未授权，请先登录")]
    Unauthorized,

    /// 禁止访问（权限不足）
    #[error("权限不足")]
    Forbidden,

    /// 资源未找到
    #[error("资源未找到: {0}")]
    NotFound(String),

    /// 资源冲突（如用户名/邮箱已存在）
    #[error("资源冲突: {0}")]
    Conflict(String),

    /// 凭证错误（用户名或密码错误）
    #[error("凭证错误: {0}")]
    InvalidCredentials(String),

    /// 校验失败（入参不合法）
    #[error("校验失败: {0}")]
    ValidationFailed(String),

    /// 触发限流/锁定
    #[error("请求过于频繁: {0}")]
    TooManyRequests(String),

    /// 内部服务器错误（数据库异常等）
    #[error("服务器内部错误: {0}")]
    InternalServerError(String),

    /// JWT 令牌相关错误
    #[error("令牌无效: {0}")]
    InvalidToken(String),
}

impl IntoResponse for AppError {
    /// 将 AppError 转换为统一格式的 JSON 错误响应
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            AppError::InvalidCredentials(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::ValidationFailed(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            AppError::InternalServerError(_) => {
                // 生产环境不暴露具体内部错误信息
                (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误".to_string())
            }
            AppError::InvalidToken(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
        };

        let body = Json(json!({
            "code": status.as_u16(),
            "message": error_message,
            "data": null,
        }));

        (status, body).into_response()
    }
}

/// 将 rust_xlsxwriter 错误转换为 AppError
impl From<rust_xlsxwriter::XlsxError> for AppError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        AppError::InternalServerError(format!("Excel 错误: {e}"))
    }
}

/// 将 anyhow::Error 转换为 AppError 的便捷实现
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::InternalServerError(err.to_string())
    }
}
