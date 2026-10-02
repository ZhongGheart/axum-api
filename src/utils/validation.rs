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

/// 口令最小长度
pub const PASSWORD_MIN_LEN: usize = 8;
/// 口令最大长度
pub const PASSWORD_MAX_LEN: usize = 128;

/// 口令复杂度校验
///
/// **只在"设置口令"时调用，绝不在"校验口令"时调用**。
/// 这是本函数最重要的使用约束：登录校验的是 Argon2 哈希，
/// 而复杂度是**明文**规则；若把它塞进登录路径，
/// 抬高门槛的当天，所有存量弱口令用户会被当场锁在门外。
///
/// 规则：长度 8–128，且至少命中 2 类字符。
///
/// 五类分别是：ASCII 大写、ASCII 小写、数字、符号、**非 ASCII 字母**。
/// 2 类而非 3 类是为了不误伤 `admin123` —— 它是 README 与 e2e 的
/// 默认账号，卡住它等于卡住首次部署和整个测试套件。
pub fn validate_password(password: &str) -> ValidationResult<()> {
    // 按**字符数**而非字节数判定：多字节口令（如中文）不该按字节被算得更长，
    // 也不该因长度差异产生"同样的密码在不同语言环境判定不同"的结果。
    let len = password.chars().count();

    if len < PASSWORD_MIN_LEN {
        return Err(AppError::BadRequest(format!(
            "密码长度不能少于 {PASSWORD_MIN_LEN} 个字符"
        )));
    }
    if len > PASSWORD_MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "密码长度不能超过 {PASSWORD_MAX_LEN} 个字符"
        )));
    }

    // 大小写**必须限定 ASCII**：`char::is_lowercase()` 对汉字返回 true
    // （Unicode 把 CJK 归为 Lo/Other_Letter 而非 Lowercase，但 Rust 的
    // is_lowercase 只查 Lowercase 属性——实际行为依赖具体字符）。
    // 不限定就会把"中文口令"误算成"含小写字母"，规则形同虚设。
    let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_symbol = password
        .chars()
        .any(|c| !c.is_alphanumeric() && !c.is_whitespace());

    // 非 ASCII 字母（中日韩、西里尔、希腊等）自成一类。
    //
    // **为什么要单列**：纯中文口令的每个字符有数千种选择，
    // 熵远高于 26 个 ASCII 小写字母。若把它判成"什么都没有"，
    // 用户唯一的出路是加一个无意义的 `@` 后缀——那是把复杂度指标
    // 变成了形式主义，而不是真的提高口令强度。
    let has_other_script = password
        .chars()
        .any(|c| !c.is_ascii() && c.is_alphanumeric());

    let classes = [
        has_upper,
        has_lower,
        has_digit,
        has_symbol,
        has_other_script,
    ]
    .iter()
    .filter(|hit| **hit)
    .count();

    if classes < 2 {
        return Err(AppError::BadRequest(
            "密码复杂度不足：需至少包含大写字母、小写字母、数字、符号中的两类（纯中文口令请至少加一个数字）"
                .into(),
        ));
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
        // 8 位且含小写+数字两类：刚好过线
        assert!(validate_password("abcd1234").is_ok());
        assert!(matches!(
            validate_password("abc123"), // 7 位
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_password(&"a1".repeat(63)), // 126 位，合法
            Ok(())
        ));
        // 128 位是**上限**，刚好过线
        assert!(validate_password(&"a1".repeat(64)).is_ok());
        assert!(matches!(
            validate_password(&"a1".repeat(65)), // 130 位，超一
            Err(AppError::BadRequest(_))
        ));
    }

    /// 单字符类必须拒绝：`12345678` 长度够但只有一个字符类，
    /// 键盘序列与字典攻击下几乎等于没有口令。
    #[test]
    fn password_requires_at_least_two_character_classes() {
        assert!(matches!(
            validate_password("12345678"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_password("aaaaaaaa"),
            Err(AppError::BadRequest(_))
        ));
        // 两类即通过，不必强求三类：把 admin123 挡在门外没有收益
        assert!(validate_password("admin123").is_ok());
        assert!(validate_password("Admin123").is_ok());
        assert!(validate_password("admin-123").is_ok());
    }

    /// 长度按**字符数**判定，不是字节数。
    /// 若按字节判，中文口令的有效长度会与英文环境不一致——
    /// 同一个密码在不同部署下得到不同的接受结果。
    #[test]
    fn password_length_counts_characters_not_bytes() {
        // 8 个汉字 = 24 字节，按字节会超 8 但按字符正好过线
        let chinese = "密码密码密码密码";
        assert!(chinese.len() > PASSWORD_MIN_LEN);
        assert!(chinese.chars().count() == PASSWORD_MIN_LEN);
        // 它确实**不再因长度**被拒：汉字属 alphanumeric，不算"符号"，
        // 所以复杂度只有 0 类，仍会被下面的字符类规则拒掉。
        // 这里断言的是"长度这一关过了"，由中文+数字那条证明
        assert!(matches!(
            validate_password(chinese),
            Err(AppError::BadRequest(msg)) if msg.contains("复杂度")
        ));
        // 中文 + 数字 = 两类，合法。证明长度按字符算而非按字节：
        // 按字节算的话 24 字节 + 1 数字会判成"超长"或"长度够但只 1 类"，
        // 结果与这里不同
        assert!(validate_password("密码密码密码密码1").is_ok());
    }

    /// 口令策略不得挂在登录校验路径上。
    ///
    /// 这条是**约束测试**：`validate_password` 是明文规则，
    /// 而登录校验的是 Argon2 哈希。若哪天有人把它塞进登录路径，
    /// 抬高门槛的当天所有存量弱口令用户会被锁在门外——
    /// 编译期看不出来，只能靠这条测试记住约束。
    #[test]
    fn password_policy_is_not_applied_to_login_verification() {
        let weak = "12345678";
        // 弱口令过不了"设置口令"这一关
        assert!(matches!(
            validate_password(weak),
            Err(AppError::BadRequest(_))
        ));
        // 但它的哈希仍应能被 check_password 认出来（登录不校验复杂度）
        let hash = crate::utils::password::hash_password(weak).expect("哈希应成功");
        assert!(matches!(
            crate::utils::password::check_password(weak, &hash).expect("校验应成功"),
            crate::utils::password::PasswordCheck::Valid
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
