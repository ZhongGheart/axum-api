//! 请求体提取器
//!
//! axum 自带的 `Json<T>` 在解析失败时**绕过 `AppError`**：
//! 直接回一个 `422 Unprocessable Entity` + `text/plain` 响应体。
//! 而本项目对外承诺统一格式 `{ code, message, data }`
//! （前端响应拦截器正是按 `message` 取文案，见 `frontend/src/api/index.ts`）。
//!
//! 结果是同一个"入参不合法"，走 `Query` 得到 400 + JSON，
//! 走 `Json` 得到 422 + 纯文本，客户端得同时处理两种形状；
//! 而纯文本那一种在拦截器里取不到 `message`，用户只能看到一个空错误。
//!
//! `ApiJson` 把 `JsonRejection` 翻译回 `AppError`，让请求体错误也走统一格式。
//!
//! **迁移状态（v0.11.0）**：目前只有本版新增的自助改密端点在用它。
//! 其余 18 处 `Json<T>` 仍是 axum 默认的 422 + 纯文本。
//! 逐个迁移是机械改动，但会让本版 diff 从"安全追溯"扩到"全站错误格式"，
//! 因此留待后续版本统一处理——**已记录在 `docs/AI_HANDOFF.md`**。

use axum::{extract::rejection::JsonRejection, extract::FromRequest, http::Request};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// 会把请求体解析失败翻译成 `AppError::BadRequest` 的 JSON 提取器
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(
        req: Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            // 逐个变体翻译，`body_text()` 保留了 serde 的原文，
            // 因此多传字段时消息里**指名是哪个字段不认**
            // （如 `unknown field \`roles\`, expected \`old_password\`...`）。
            // 这与 `From<QueryRejection>` 对查询参数的处理是同一个目的：
            // 让入参错误响着失败，而不是给一句无从下手的"解析失败"
            Err(rejection) => Err(map_rejection(rejection)),
        }
    }
}

/// 把 `JsonRejection` 的四种变体翻译成统一响应
fn map_rejection(rejection: JsonRejection) -> AppError {
    let detail = match &rejection {
        JsonRejection::JsonDataError(e) => e.body_text(),
        JsonRejection::JsonSyntaxError(e) => e.body_text(),
        JsonRejection::MissingJsonContentType(_) => {
            "请求必须带 Content-Type: application/json".to_string()
        }
        // 读取请求体失败（如客户端提前断开）：这是传输层问题，
        // 不是入参写错，按 400 回会误导调用方以为是自己的问题
        JsonRejection::BytesRejection(e) => {
            return AppError::BadRequest(format!("读取请求体失败: {}", e.body_text()))
        }
        // `JsonRejection` 是 `#[non_exhaustive]`：axum 小版本新增变体时
        // 这里不会编译失败，只会走统一格式的兜底文案——
        // 与其让升级直接编译不过，不如在这里显式承认"未知形态"
        other => {
            return AppError::BadRequest(format!("请求体不合法（未识别的拒绝类型）: {other:?}"))
        }
    };
    AppError::BadRequest(format!("请求体不合法: {detail}"))
}

impl<T> std::ops::Deref for ApiJson<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for ApiJson<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
