//! 会把入参解析失败翻译回 `AppError` 的提取器
//!
//! axum 自带的 `Json<T>` / `Path<T>` 在解析失败时**绕过 `AppError`**：
//! 直接回一个 `400`/`422` + `text/plain` 响应体。
//! 而本项目对外承诺统一格式 `{ code, message, data }`
//! （前端响应拦截器正是按 `message` 取文案，见 `frontend/src/api/index.ts`）。
//!
//! 结果是同一个"入参不合法"，走 `Query` 得到 400 + JSON，
//! 走 `Json` 得到 400 + 纯文本，走 `Path` 得到 400 + 纯文本，
//! 客户端得同时处理两种形状；
//! 而纯文本那一种在拦截器里取不到 `message`，用户只能看到一个空错误。
//!
//! `ApiJson` 把 `JsonRejection` 翻译回 `AppError`，
//! `ApiPath` 把 `PathRejection` 翻译回 `AppError`，让请求体与路径参数错误也走统一格式。
//!
//! **迁移状态（v0.12.0）**：全站 `Json<T>` 与 `Path<T>` 提取器已迁移完毕，
//! 由 `tests/api_integration.rs` 的 `every_bad_input_returns_unified_error_envelope` 承重——
//! 它遍历 OpenAPI 全路由实测响应形状，新增端点忘了用本模块的提取器会当场变红。
//! 因此不要用"源码里还有没有裸 `Json<`"这类文本断言代替它。
//!
//! 有意保留 axum 原生提取器的地方：`swagger_ui_handler` 的 `Path<String>`
//! 属于 SPA 静态资源回退路由，失败时该回 HTML/404，不是 JSON 信封。

use axum::{
    extract::rejection::{JsonRejection, PathRejection},
    extract::{FromRequest, FromRequestParts},
    http::{request::Parts, Request},
};
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

/// 会把路径参数解析失败翻译成 `AppError::BadRequest` 的路径提取器
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiPath<T>(pub T);

/// `Path<T>` 实现的 trait 是 `FromRequestParts` 而非 `FromRequest`：
/// 路径参数不消费请求体，axum 因此允许从 `Parts` 里取
impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(ApiPath(value)),
            Err(rejection) => Err(map_path_rejection(rejection)),
        }
    }
}

/// 把 `PathRejection` 的两种变体翻译成统一响应
fn map_path_rejection(rejection: PathRejection) -> AppError {
    let detail = match &rejection {
        // 最常见的一种：`/api/admin/users/not-a-uuid` 把 `{id}` 塞了非 UUID，
        // 或路由声明的参数与实际不符。`body_text()` 保留 serde 原文，
        // 因此消息里能指名是哪个参数不合法，而不是一句无从下手的"解析失败"
        PathRejection::FailedToDeserializePathParams(e) => e.body_text(),
        // 路由声明了 `{id}` 但请求里没匹配到：属于路由与文档不一致，
        // 说清这一点比让调用方以为自己传错了更有用
        PathRejection::MissingPathParams(_) => {
            "请求缺少路径参数（路由与文档声明不一致）".to_string()
        }
        // `PathRejection` 同样是 `#[non_exhaustive]`：axum 小版本新增变体时
        // 这里不会编译失败，只会走统一格式的兜底文案。理由同 `map_rejection`
        other => {
            return AppError::BadRequest(format!("路径参数不合法（未识别的拒绝类型）: {other:?}"))
        }
    };
    AppError::BadRequest(format!("路径参数不合法: {detail}"))
}

impl<T> std::ops::Deref for ApiPath<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for ApiPath<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
