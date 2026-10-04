//! 批量导入用户的 CSV 解析
//!
//! 只做**解析与结构校验**，不碰数据库、不写用户。理由与 `validation` 一致：
//! 归一化、复杂度、唯一性这些规则只有一份实现（`validation::normalize_*`
//! 与 `create_user`），导入若自己写一套，第二年两套规则必然走偏。
//!
//! ── 为什么行级失败不整批中止 ──
//!
//! 管理员导入的是一份可能有 200 行的表，里面错了一行就整批回滚的话，
//! 他既不知道错在哪一行，也没法只补那一个用户。解析结果因此是
//! "逐行带上行号与原因"，由调用方决定逐行入库。

use crate::error::AppError;

/// 解析出来的单行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUserRow {
    /// CSV 中的行号（**从 1 开始，且含表头行**），用于回报告诉人错在哪
    pub line: usize,
    pub username: String,
    pub email: String,
    pub password: String,
    /// 可选展示名
    pub display_name: Option<String>,
    /// 角色名列表
    pub roles: Vec<String>,
}

/// 单批最多多少行
///
/// 没有上限的话，一个 100 万行的文件会让单次请求持有 100 万个待插入用户，
/// 事务时长、锁持有时间与内存都随文件线性增长，且请求永不返回。
pub const MAX_IMPORT_ROWS: usize = 1000;

/// 必须存在的列
const REQUIRED_HEADERS: &[&str] = &["username", "email", "password", "roles"];

/// 解析用户导入 CSV
///
/// 表头列名大小写不敏感；`roles` 单元格内部用 `|` 分隔多个角色
/// （`user|admin`）。**不用逗号**是因为逗号是 CSV 本身的分隔符，
/// 让用户手写引号转义是种折磨，而引号写漏了会静默产生一个叫
/// `user,admin` 的不存在的角色。
pub fn parse_user_csv(text: &str) -> Result<Vec<ParsedUserRow>, AppError> {
    // Excel 导出的 CSV 常带 UTF-8 BOM。不剥掉的话第一个列名会变成
    // "\u{feff}username"，于是"表头缺 username"的报错出现在一个
    // 明明从 Excel 导出的文件上——这是最难自查的一类问题。
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| AppError::BadRequest(format!("CSV 表头无法解析: {e}")))?
        .clone();

    let index_of = |name: &str| -> Option<usize> {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };

    let missing: Vec<&str> = REQUIRED_HEADERS
        .iter()
        .filter(|h| index_of(h).is_none())
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(AppError::BadRequest(format!(
            "CSV 缺少必需的列：{}。表头应为 {}",
            missing.join("、"),
            REQUIRED_HEADERS.join(",")
        )));
    }

    let idx_username = index_of("username").unwrap();
    let idx_email = index_of("email").unwrap();
    let idx_password = index_of("password").unwrap();
    let idx_roles = index_of("roles").unwrap();
    let idx_display = index_of("display_name");

    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        // +2：枚举从 0 开始，且第 0 行是表头
        let line = i + 2;
        let record =
            record.map_err(|e| AppError::BadRequest(format!("第 {line} 行无法解析: {e}")))?;

        let get = |idx: usize| record.get(idx).unwrap_or("").trim().to_string();

        // 整行空白：Excel 导出常见尾部空行，不该报"缺少用户名"
        if record.iter().all(|f| f.trim().is_empty()) {
            continue;
        }

        if rows.len() >= MAX_IMPORT_ROWS {
            return Err(AppError::BadRequest(format!(
                "单批最多导入 {MAX_IMPORT_ROWS} 行，超出部分已忽略；请分批导入"
            )));
        }

        let username = get(idx_username);
        let email = get(idx_email);
        let password = get(idx_password);
        let display_name = idx_display.map(&get).filter(|s| !s.is_empty());
        let roles: Vec<String> = get(idx_roles)
            .split('|')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        rows.push(ParsedUserRow {
            line,
            username,
            email,
            password,
            display_name,
            roles,
        });
    }

    if rows.is_empty() {
        return Err(AppError::BadRequest(
            "CSV 里没有任何数据行（只有表头也算空）".to_string(),
        ));
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_csv_parses_into_rows() {
        let csv = "username,email,password,roles\nalice,a@x.com,Pw123456!,user\n";
        let rows = parse_user_csv(csv).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line, 2);
        assert_eq!(rows[0].username, "alice");
        assert_eq!(rows[0].roles, vec!["user"]);
        assert!(rows[0].display_name.is_none());
    }

    #[test]
    fn a_missing_required_column_is_named_in_the_error() {
        let csv = "username,email\nalice,a@x.com\n";
        let err = parse_user_csv(csv).unwrap_err().to_string();
        assert!(err.contains("password"), "错误必须指名缺哪列: {err}");
        assert!(err.contains("roles"), "错误必须指名缺哪列: {err}");
    }

    #[test]
    fn the_byte_order_mark_does_not_hide_the_first_column() {
        let csv = "\u{feff}username,email,password,roles\nalice,a@x.com,Pw123456!,user\n";
        let rows = parse_user_csv(csv).unwrap();
        assert_eq!(rows[0].username, "alice");
    }

    #[test]
    fn blank_trailing_lines_are_skipped_but_line_numbers_stay_true() {
        let csv = "username,email,password,roles\nalice,a@x.com,Pw123456!,user\n\n\n";
        let rows = parse_user_csv(csv).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line, 2);
    }

    #[test]
    fn roles_split_on_pipe_not_comma() {
        let csv = "username,email,password,roles\nbob,b@x.com,Pw123456!,user | admin\n";
        let rows = parse_user_csv(csv).unwrap();
        assert_eq!(rows[0].roles, vec!["user", "admin"]);
    }

    #[test]
    fn a_header_only_file_is_refused() {
        let csv = "username,email,password,roles\n";
        assert!(parse_user_csv(csv).is_err());
    }

    #[test]
    fn headers_are_matched_case_insensitively() {
        let csv = "UserName,Email,Password,Roles\ncarol,c@x.com,Pw123456!,user\n";
        let rows = parse_user_csv(csv).unwrap();
        assert_eq!(rows[0].username, "carol");
    }
}
