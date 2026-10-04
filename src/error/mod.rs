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

    /// 权限码不足（细粒度授权拦截，附带缺失的权限码）
    #[error("缺少权限：{0}")]
    PermissionDenied(String),

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

    /// 上传体积超限
    ///
    /// 与 `BadRequest` 分开是因为客户端要据此改行为（重新选一个小一点的图），
    /// 而 400 的文案它通常理解成"参数写错了"。
    #[error("上传内容过大: {0}")]
    PayloadTooLarge(String),

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
            AppError::PermissionDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            AppError::InvalidCredentials(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::ValidationFailed(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            AppError::InternalServerError(detail) => {
                // 具体原因不回传客户端，但必须留在服务端日志里，否则 500 无法定位
                tracing::error!("内部错误: {detail}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器内部错误".to_string(),
                )
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
/// 把 `Query` 提取器的拒绝翻译成统一响应格式
///
/// **为什么需要它**：`Query<T>` 解析失败时，axum 默认回一个 `text/plain`
/// 的 400，**绕过 `AppError`**。而本项目对外承诺统一响应格式
/// `{ code, message, data }`——前端拦截器正是按 `message` 取文案。
/// 于是同一个 400，有的走统一格式有的不走，客户端得两种都处理。
///
/// 消息里保留了 serde 的原文，因此会**指名是哪个字段不认**
/// （如 `unknown field \`departmnt\`, expected one of \`page\`, ...`），
/// 这正是让"筛选参数写错"从静默失效变成响着失败的关键。
impl From<axum::extract::rejection::QueryRejection> for AppError {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        AppError::BadRequest(format!("查询参数不合法: {rejection}"))
    }
}

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
