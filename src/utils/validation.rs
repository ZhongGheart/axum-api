//! 通用数据校验工具
//!
//! 基于 validator crate 提供复杂参数校验规则封装。

use crate::error::AppError;

/// 校验结果类型
pub type ValidationResult<T> = Result<T, AppError>;

/// 用户名校验（3-50 字符，字母数字下划线）
pub fn validate_username(username: &str) -> ValidationResult<()> {
    if username.len() < 3 || username.len() > 50 {
        return Err(AppError::BadRequest(
            "用户名长度必须在 3-50 个字符之间".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "用户名只能包含字母、数字、下划线和连字符".into(),
        ));
    }
    Ok(())
}

/// 密码校验（至少 6 位）
pub fn validate_password(password: &str) -> ValidationResult<()> {
    if password.len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于 6 个字符".into()));
    }
    if password.len() > 128 {
        return Err(AppError::BadRequest("密码长度不能超过 128 个字符".into()));
    }
    Ok(())
}

/// 转义 LIKE 模式里的特殊字符
///
/// **必须转义**，否则用户搜 `100%` 会匹配到全部记录（`%` 在 LIKE 里是通配符），
/// 搜 `a_b` 也会误中 `axb`。转义后要在 SQL 里配 `ESCAPE '\'`。
///
/// 反斜杠要**最先**转义：若先转义 `%` / `_`，它们产生的反斜杠会被后续步骤
/// 再转义一次，变成 `\\%`，LIKE 读到的是字面反斜杠加通配符——漏洞照旧。
pub fn escape_like_pattern(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            _ => out.push(ch),
        }
    }
    out
}

/// 邮箱基本校验
pub fn validate_email(email: &str) -> ValidationResult<()> {
    if !email.contains('@') || !email.contains('.') {
        return Err(AppError::BadRequest("邮箱格式不正确".into()));
    }
    if email.len() > 255 {
        return Err(AppError::BadRequest("邮箱长度不能超过 255 个字符".into()));
    }
    Ok(())
}

/// 分页参数校验
pub fn validate_page(page: i64, page_size: i64) -> ValidationResult<()> {
    if page < 1 {
        return Err(AppError::BadRequest("页码必须大于 0".into()));
    }
    if !(1..=200).contains(&page_size) {
        return Err(AppError::BadRequest("每页条数必须在 1-200 之间".into()));
    }
    Ok(())
}

/// UUID 格式校验
pub fn validate_uuid(s: &str) -> ValidationResult<()> {
    if uuid::Uuid::parse_str(s).is_err() {
        return Err(AppError::BadRequest("ID 格式不正确".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_pattern_escapes_wildcards() {
        // 不转义的话，搜 "100%" 会匹配到全部记录
        assert_eq!(escape_like_pattern("100%"), r"100\%");
        // 下划线同样是通配符：a_b 会误中 axb
        assert_eq!(escape_like_pattern("a_b"), r"a\_b");
    }

    /// 反斜杠必须最先转义，否则它自己产生的反斜杠会被再转义一次，
    /// LIKE 读到的就是字面反斜杠 + 通配符 —— 漏洞照旧。
    #[test]
    fn like_pattern_escapes_the_escape_character_itself() {
        assert_eq!(escape_like_pattern(r"a\%b"), r"a\\\%b");
        assert_eq!(escape_like_pattern(r"\"), r"\\");
    }

    #[test]
    fn like_pattern_leaves_ordinary_text_untouched() {
        assert_eq!(escape_like_pattern("alice"), "alice");
        assert_eq!(escape_like_pattern(""), "");
    }

    #[test]
    fn username_accepts_valid_values() {
        assert!(validate_username("alice_01").is_ok());
        assert!(validate_username("alice-01").is_ok());
    }

    #[test]
    fn username_rejects_invalid_length_and_characters() {
        assert!(matches!(
            validate_username("ab"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_username("alice space"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_username(&"a".repeat(51)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn password_enforces_length_boundaries() {
        assert!(validate_password("123456").is_ok());
        assert!(matches!(
            validate_password("12345"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_password(&"a".repeat(129)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn email_requires_basic_shape_and_length_boundary() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(matches!(
            validate_email("user.example.com"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_email(&format!("{}@example.com", "a".repeat(250))),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn page_validation_enforces_bounds() {
        assert!(validate_page(1, 1).is_ok());
        assert!(validate_page(10, 200).is_ok());
        assert!(matches!(validate_page(0, 10), Err(AppError::BadRequest(_))));
        assert!(matches!(validate_page(1, 0), Err(AppError::BadRequest(_))));
        assert!(matches!(
            validate_page(1, 201),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn uuid_validation_accepts_generated_uuid() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(validate_uuid(&id).is_ok());
        assert!(matches!(
            validate_uuid("not-a-uuid"),
            Err(AppError::BadRequest(_))
        ));
    }
}
