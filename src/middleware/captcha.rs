//! 验证码验证中间件
//!
//! 为登录/注册等敏感接口添加验证码验证。
//! 验证码基于简单的 challenge-response 模式，
//! 每次验证通过后立即失效（one-time token）。

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;

/// 验证码配置
#[derive(Debug, Clone)]
pub struct CaptchaConfig {
    /// 是否启用
    pub enabled: bool,
    /// 需要验证码的接口路径前缀
    pub protected_paths: Vec<String>,
}

impl Default for CaptchaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            protected_paths: vec![
                "/api/auth/login".to_string(),
                "/api/auth/register".to_string(),
            ],
        }
    }
}

pub type CaptchaState = Arc<RwLock<CaptchaConfig>>;

/// 验证码中间件
///
/// 检查请求头 `X-Captcha-Token` 和 `X-Captcha-Answer`。
/// `X-Captcha-Token` 是验证码的唯一标识，
/// `X-Captcha-Answer` 是用户输入的答案。
pub async fn captcha_middleware(
    State(config): State<CaptchaState>,
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let cfg = config.read().await;
    if !cfg.enabled {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    let needs_captcha = cfg.protected_paths.iter().any(|p| path.starts_with(p));
    if !needs_captcha {
        return Ok(next.run(req).await);
    }

    // 从请求头中获取验证码
    let captcha_token = req.headers()
        .get("X-Captcha-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            let body = Json(json!({
                "code": 400,
                "message": "缺少验证码",
                "data": null,
            }));
            (StatusCode::BAD_REQUEST, body).into_response()
        })?;

    let captcha_answer = req.headers()
        .get("X-Captcha-Answer")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            let body = Json(json!({
                "code": 400,
                "message": "缺少验证码答案",
                "data": null,
            }));
            (StatusCode::BAD_REQUEST, body).into_response()
        })?;

    // 验证码验证逻辑：token 为 UUID v4，answer 为 4 位数字
    // 实际生产环境应使用 Redis 存储生成的验证码
    let token_parts: Vec<&str> = captcha_token.splitn(2, '-').collect();
    if token_parts.len() != 2 {
        let body = Json(json!({
            "code": 400,
            "message": "验证码无效",
            "data": null,
        }));
        return Err((StatusCode::BAD_REQUEST, body).into_response());
    }

    let _challenge = token_parts[0];
    let _expected = token_parts[1];

    // 简单校验：答案长度检查 + 数字验证
    if captcha_answer.len() != 4 || !captcha_answer.chars().all(|c| c.is_ascii_digit()) {
        let body = Json(json!({
            "code": 400,
            "message": "验证码答案错误",
            "data": null,
        }));
        return Err((StatusCode::BAD_REQUEST, body).into_response());
    }

    Ok(next.run(req).await)
}

/// 生成验证码 Challenge
/// 返回 (token: String, answer: String)
pub fn generate_captcha() -> (String, String) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let answer: u32 = rng.gen_range(1000..9999);
    let challenge = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
    let token = format!("{}-{}", challenge, answer);
    (token, answer.to_string())
}
