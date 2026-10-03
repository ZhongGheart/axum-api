//! 审计摘要的格式化助手
//!
//! v0.13.0 起，写操作的 handler 往 [`crate::middleware::audit_log::AuditDetail`]
//! 里追加"改了什么"。本模块只负责把这些事实**排成固定格式**，
//! 使同一个资源在不同端点（新建/更新/删除）里的说法一致——
//! 审计要在乎"事后能不能搜"，格式不统一就等于没法搜。
//!
//! ## 为什么删除前必须先查名字
//!
//! `roles` / `users` / `dict_types` 的名字都只存在行里，行删掉就没了。
//! 此前 `DELETE /api/admin/roles/{id}` 只在审计里留下一个 UUID，
//! 而 UUID 指向的行已经不存在——事后连"删的是什么角色"都答不出来。
//! 因此删除类 handler 要在 `DELETE` 之前把名字读出来（见
//! [`role_label`] / [`user_label`] / [`menu_label`] / [`dict_type_label`]）。

use crate::error::AppError;
use crate::router::AppState;
use uuid::Uuid;

/// 权限码清单在摘要里的最大展示条数
///
/// 角色可能挂几十个菜单，全量列出来会让一条审计记录长到没法在界面上看。
/// 超出的部分用"N 个"概括——**概括也要说清总数**，否则读者会以为只有这些。
const MAX_CODES_SHOWN: usize = 12;

/// 资源标签：`角色 "admin"`
///
/// 用 ASCII 双引号而不是中文引号：审计摘要常被复制到工单、聊天窗口、
/// SQL 查询里，`"..."` 在这些地方不会和中文标点混淆。
pub fn label(kind: &str, name: &str) -> String {
    format!("{kind} \"{name}\"")
}

/// 标签的降级形式：`角色 <uuid>`
///
/// 只在查名字失败时用。相比丢掉标签只留裸 UUID，
/// 带上 UUID 至少还能让人把这条记录和当时的请求对上。
pub fn label_or_id(kind: &str, name: Option<&str>, id: Uuid) -> String {
    match name {
        Some(name) => label(kind, name),
        None => format!("{kind} <{id}>"),
    }
}

/// 角色名（删除/改名前查一次）
pub async fn role_label(state: &AppState, id: Uuid) -> String {
    match state.auth_service.role_repo.find_name_by_id(id).await {
        Ok(Some(name)) => label("角色", &name),
        Ok(None) => label_or_id("角色", None, id),
        Err(e) => lookup_failed("角色", id, &e),
    }
}

/// 用户名（删除/停用/重置口令前查一次）
pub async fn user_label(state: &AppState, id: Uuid) -> String {
    match state.auth_service.user_repo.find_by_id(id).await {
        Ok(user) => label("用户", &user.username),
        Err(e) => lookup_failed("用户", id, &e),
    }
}

/// 菜单名（删除/改权限码前查一次）
pub async fn menu_label(state: &AppState, id: Uuid) -> String {
    match state.menu_repo.find_by_id(id).await {
        Ok(menu) => label("菜单", &menu.name),
        Err(e) => lookup_failed("菜单", id, &e),
    }
}

/// 字典类型标识（删除前查一次）
pub async fn dict_type_label(state: &AppState, id: Uuid) -> String {
    match state.dict_repo.find_type_by_id(id).await {
        Ok(t) => label("字典类型", &t.code),
        Err(e) => lookup_failed("字典类型", id, &e),
    }
}

/// 字典项标识（删除前查一次）
pub async fn dict_item_label(state: &AppState, id: Uuid) -> String {
    match state.dict_repo.find_item_by_id(id).await {
        Ok(item) => label("字典项", &format!("{}={}", item.label, item.value)),
        Err(e) => lookup_failed("字典项", id, &e),
    }
}

/// 查名字失败时的统一退路
///
/// **不让它冒泡成 500**：审计摘要是旁路能力，
/// 一次查询失败不该让本来能成功的删除请求整个失败。
/// 退化成 `角色 <uuid>` 而不是丢掉标签——
/// 丢掉之后这条记录就只剩一个孤零零的路径和 UUID，谁也拼不回去。
fn lookup_failed(kind: &str, id: Uuid, e: &AppError) -> String {
    tracing::warn!("审计摘要：查{kind}名失败（{id}）: {e}");
    label_or_id(kind, None, id)
}

/// 把一组权限码排成摘要片段：`授予权限码 system:a、system:b`
///
/// 空集合返回空串，调用方据此**整条省略**这个片段——
/// "授予了 0 个权限码"这种话写进审计只会让人以为发生过什么。
pub fn codes(prefix: &str, codes: &[String]) -> String {
    if codes.is_empty() {
        return String::new();
    }
    if codes.len() <= MAX_CODES_SHOWN {
        return format!("{prefix} {}", codes.join("、"));
    }
    let shown = codes[..MAX_CODES_SHOWN].join("、");
    format!("{prefix} {shown} 等 {} 个", codes.len())
}

/// 角色清单摘要：`角色 admin、auditor`
pub fn roles_list(roles: &[String]) -> String {
    if roles.is_empty() {
        return "无角色".to_string();
    }
    roles.join("、")
}

/// 把两个集合的差异排成一段可读的变更摘要
///
/// 两边都没变时返回空串——重复提交同一份授权不是"变更"，
/// 把它记成变更会让"谁动过这里"这个问题多出假阳性。
///
/// 授权类操作要回答的是"这次**变了什么**"，不是"现在的完整集合是什么"——
/// 后者看着信息更多，但审计是长期留存物，读者真正要找的是差异。
pub fn diff_summary(
    before: &[String],
    after: &[String],
    granted_prefix: &str,
    revoked_prefix: &str,
) -> String {
    let added = after
        .iter()
        .filter(|c| !before.contains(c))
        .cloned()
        .collect::<Vec<_>>();
    let removed = before
        .iter()
        .filter(|c| !after.contains(c))
        .cloned()
        .collect::<Vec<_>>();
    let granted = codes(granted_prefix, &added);
    let revoked = codes(revoked_prefix, &removed);
    [granted, revoked]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("；")
}

/// 权限码变化的摘要：`菜单 "x" 的权限码由 "a" 改为 "b"`
///
/// ## 抽成纯函数的原因：`a → b` 这条路在接口层根本走不通
///
/// `update_menu` 有两道各自正确的守卫，合在一起把"改码"逼成了死路：
///
/// 1. **改写权限码要求调用者已持有目标码**（`ensure_covers`）。
///    而一个全新的码只能来自"新建按钮菜单"——那条路不要求持有，
///    随后**授给自己的角色**也会被自授守卫拦住（授给别的角色才行）。
///    于是"自己声明一个码、再把它改到另一个按钮上"始终缺一个持有者。
/// 2. **目标码已存在时撞唯一索引**（迁移 `007`）返回 409。
///    目标码对应的菜单一旦存在，唯一索引就挡住第二处声明。
///
/// 结果：实践中改码只能"清空 → 恢复 / 新建菜单"，
/// 这个函数的 `from`/`to` 都非空分支是**防御性**的——
/// 守卫或索引哪天放开时它立刻生效，在那之前接口测试永远走不到。
/// 与其删掉它（等放开那天再写一遍，且大概率写错），
/// 不如在这里用单测钉住它排出的字符串。
pub fn permission_change(
    label: &str,
    id: Uuid,
    before: Option<&str>,
    after: Option<&str>,
) -> String {
    if before == after {
        return format!("更新{label}（{id}）");
    }
    let from = before.unwrap_or("无");
    let to = after.unwrap_or("无");
    format!("{label}的权限码由 \"{from}\" 改为 \"{to}\"（{id}）")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(code: &str) -> String {
        code.to_string()
    }

    #[test]
    fn empty_grant_list_produces_no_fragment() {
        // "授予了 0 个权限码"会让人以为发生过什么，必须整条省略
        assert_eq!(codes("授予权限码", &[]), "");
        assert_eq!(diff_summary(&[], &[], "授予", "撤销"), "");
    }

    #[test]
    fn diff_summary_reports_only_what_changed() {
        let before = vec![v("system:user:list"), v("system:role:list")];
        let after = vec![v("system:user:list"), v("system:log:list")];
        let summary = diff_summary(&before, &after, "授予权限码", "撤销权限码");
        assert_eq!(
            summary,
            "授予权限码 system:log:list；撤销权限码 system:role:list"
        );
        // 没变的那个码不该出现在任何一段里
        assert_eq!(summary.matches("system:user:list").count(), 0);
    }

    #[test]
    fn identical_sets_are_not_a_change() {
        let roles = vec![v("admin"), v("user")];
        assert_eq!(diff_summary(&roles, &roles, "追加角色", "移除角色"), "");
    }

    #[test]
    fn long_code_lists_are_summarised_but_keep_the_total() {
        let many = (0..20)
            .map(|i| v(&format!("system:m{i}:list")))
            .collect::<Vec<_>>();
        let summary = codes("授予权限码", &many);
        assert!(summary.contains("等 20 个"), "必须说清总数: {summary}");
        assert_eq!(summary.matches("system:").count(), MAX_CODES_SHOWN);
    }

    #[test]
    fn labels_fall_back_to_the_id_when_the_name_is_gone() {
        let id = Uuid::nil();
        assert_eq!(label("角色", "admin"), "角色 \"admin\"");
        assert_eq!(
            label_or_id("角色", None, id),
            "角色 <00000000-0000-0000-0000-000000000000>"
        );
    }

    #[test]
    fn permission_change_keeps_both_sides() {
        let id = Uuid::nil();
        // 这条分支接口层走不到（见函数说明），但摘要必须是对的
        assert_eq!(
            permission_change("菜单 \"用户管理\"", id, Some("a:read"), Some("a:write")),
            "菜单 \"用户管理\"的权限码由 \"a:read\" 改为 \"a:write\"（00000000-0000-0000-0000-000000000000）"
        );
        // 清空是可达路径：必须说成"改为无"，不能还留着旧码让人以为按钮仍有权限
        assert!(permission_change("菜单 \"x\"", id, Some("a:read"), None).contains("改为 \"无\""));
        // 没动码时不谎称改过
        assert_eq!(
            permission_change("菜单 \"x\"", id, Some("a:read"), Some("a:read")),
            "更新菜单 \"x\"（00000000-0000-0000-0000-000000000000）"
        );
    }

    #[test]
    fn empty_role_list_is_spelled_out() {
        // 空白会让"这个账号没有任何角色"看起来像没记录
        assert_eq!(roles_list(&[]), "无角色");
        assert_eq!(roles_list(&[v("admin"), v("user")]), "admin、user");
    }
}
