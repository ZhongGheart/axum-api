//! 全局统一响应封装
//!
//! 提供 `ApiResponse<T>` 结构体，确保所有 API 返回一致的 JSON 格式。

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

/// 统一 API 响应结构
///
/// 所有接口统一返回此格式：
/// ```json
/// {
///   "code": 200,
///   "message": "success",
///   "data": { ... }
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    /// 状态码，与 HTTP 状态码一致
    pub code: u16,
    /// 提示信息
    pub message: String,
    /// 响应数据
    pub data: Option<T>,
}

#[allow(dead_code)]
impl<T: Serialize> ApiResponse<T> {
    /// 创建成功响应（带数据）
    pub fn success(data: T) -> Self {
        Self {
            code: 200,
            message: "success".to_string(),
            data: Some(data),
        }
    }

    /// 创建成功响应（无数据）
    pub fn success_no_data() -> Self {
        Self {
            code: 200,
            message: "success".to_string(),
            data: None,
        }
    }

    /// 创建错误响应
    pub fn error(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// 实现 IntoResponse 使 ApiResponse 可直接作为 Axum 处理器返回值
impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}
