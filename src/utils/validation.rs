//! 通用数据校验工具
//!
//! 基于 validator crate 提供复杂参数校验规则封装。

use crate::error::AppError;

/// 校验结果类型
pub type ValidationResult<T> = Result<T, AppError>;

/// 用户名校验（3-50 字符，字母数字下划线）
pub fn validate_username(username: &str) -> ValidationResult<()> {
    if username.len() < 3 || username.len() > 50 {
        return Err(AppError::BadRequest("用户名长度必须在 3-50 个字符之间".into()));
    }
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(AppError::BadRequest("用户名只能包含字母、数字、下划线和连字符".into()));
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
    if page_size < 1 || page_size > 200 {
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

/// 手机号格式校验（中国大陆）
pub fn validate_phone(phone: &str) -> ValidationResult<()> {
    if phone.len() != 11 || !phone.starts_with('1') {
        return Err(AppError::BadRequest("手机号格式不正确".into()));
    }
    if !phone.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest("手机号只能包含数字".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_accepts_valid_values() {
        assert!(validate_username("alice_01").is_ok());
        assert!(validate_username("alice-01").is_ok());
    }

    #[test]
    fn username_rejects_invalid_length_and_characters() {
        assert!(matches!(validate_username("ab"), Err(AppError::BadRequest(_))));
        assert!(matches!(validate_username("alice space"), Err(AppError::BadRequest(_))));
        assert!(matches!(
            validate_username(&"a".repeat(51)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn password_enforces_length_boundaries() {
        assert!(validate_password("123456").is_ok());
        assert!(matches!(validate_password("12345"), Err(AppError::BadRequest(_))));
        assert!(matches!(
            validate_password(&"a".repeat(129)),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn email_requires_basic_shape_and_length_boundary() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(matches!(validate_email("user.example.com"), Err(AppError::BadRequest(_))));
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
        assert!(matches!(validate_page(1, 201), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn uuid_validation_accepts_generated_uuid() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(validate_uuid(&id).is_ok());
        assert!(matches!(validate_uuid("not-a-uuid"), Err(AppError::BadRequest(_))));
    }

    #[test]
    fn phone_validation_requires_mainland_mobile_shape() {
        assert!(validate_phone("13800138000").is_ok());
        assert!(matches!(validate_phone("1380013800"), Err(AppError::BadRequest(_))));
        assert!(matches!(validate_phone("23800138000"), Err(AppError::BadRequest(_))));
        assert!(matches!(validate_phone("1380013800a"), Err(AppError::BadRequest(_))));
    }
}
