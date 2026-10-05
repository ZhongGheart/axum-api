//! 操作日志中间件
//!
//! 把已认证请求的操作信息异步写入 `audit_logs`。
//!
//! 设计取舍：
//! - 只记录方法、路径、查询串与 handler 声明的**变更摘要**，
//!   **不记录请求体**。登录口令、令牌等敏感字段
//!   一旦入库就是长期泄露面；同时读取请求体会破坏下游 extractor。
//! - 该中间件注册在认证中间件之内，因此能拿到已认证用户；
//!   未认证请求不进入受保护路由，也就不会产生审计记录。
//!
//! ## v0.13.0：写操作必须留下"改了什么"
//!
//! 此前 `result` 列对**所有写操作恒为空**，于是审计只能回答
//! 「谁在什么时候调了哪个接口」，回答不了「改了什么」。实测后果有两个：
//! 角色被删后名字永久丢失（`roles` 行已删，审计只剩一个 UUID），
//! 授权授予无法复盘（不知道授了/撤了哪些权限码）。
//!
//! 修法是 [`AuditDetail`]：handler 显式声明**它自己知道安全的那部分**，
//! 中间件在写库前合并进 `result`。
//! 不自动记录请求体是因为那会把 `password` / `old_password` 写进长期表——
//! 不写就不入库，默认安全而不是默认危险。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    extract::{FromRequestParts, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::middleware::auth::AuthenticatedUser;
use crate::middleware::client_ip;
use crate::model::{AuditTargetEntry, ChangeType, TargetType};
use crate::router::AppState;

/// 查询串入库前的最大长度
const MAX_QUERY_LEN: usize = 500;

/// `action` 入库前的最大长度
///
/// 与迁移 015 里的列宽一致。两边必须同步：列窄于此则超长路径的审计
/// 会被 Postgres 拒掉（且不报错），列宽于此则这里的截断是多余的安全网。
const MAX_ACTION_LEN: usize = 512;

/// 摘要入库前的最大长度
///
/// `result` 是无长度限制的 `TEXT`，而摘要可能包含几十个权限码。
/// 不设上限的话，一条"把全部菜单授予角色"的记录能写进几十 KB，
/// 日志表是本库唯一无限增长的表，这样喂它等于自己给自己埋雷。
const MAX_RESULT_LEN: usize = 2000;

/// 多条摘要合并进 `result` 时的分隔符
///
/// 用可读的 `；` 而不是 JSON：`result` 是给人读的（导出成 Excel 的
/// "操作日志"表里直接展示），不是给程序解析的。
const DETAIL_SEPARATOR: &str = "；";

/// `target_label` 入库前的最大长度
///
/// 与迁移 020 里的列宽（VARCHAR(200)）一致。名字来自库里的真实值，
/// 但列窄于此而这里不截断，超长名字会让 INSERT 失败——
/// 在 `tokio::spawn` 路径上那就是**整条审计连同它的 target 一起消失**，
/// 而请求照常 200。所以两边必须同步。
const MAX_TARGET_LABEL_LEN: usize = 200;

/// `target_key` 入库前的最大长度（与迁移 020 的列宽一致，理由同上）
const MAX_TARGET_KEY_LEN: usize = 200;

/// 写操作摘要累加器
///
/// 由中间件在请求进入时挂到 extensions 上，handler 通过
/// `FromRequestParts` 取同一个 `Arc` 往里追加，中间件在写库前合并成
/// `result` 文本。用 `Arc<Mutex<..>>` 而不是 `RefCell` 是因为
/// handler 是 `async` 的，`Mutex` 的 guard 不能跨 `.await` 持有——
/// 所以只在 `push` 内部短暂加锁，从不把 guard 拿出函数。
#[derive(Debug, Clone, Default)]
pub struct AuditDetail {
    lines: Arc<Mutex<Vec<String>>>,
    /// 结构化对象引用（v0.26.0）
    ///
    /// 与 `lines` **并行**：文本给人读，这里给机器筛。
    targets: Arc<Mutex<Vec<AuditTargetEntry>>>,
}

impl AuditDetail {
    /// 追加一条摘要
    ///
    /// **只在副作用真正落库之后调用**。中间件只对 2xx 响应合并摘要
    /// （见 [`audit_log_middleware`]），但"写库成功、后续步骤失败"
    /// 仍会返回 5xx 而丢掉摘要——那个方向是**少记**，不是**错记**，
    /// 本模块宁可少说也不肯说错，与 v0.10.0「停止说谎」一致。
    pub fn push(&self, line: impl Into<String>) {
        self.push_inner(line, None);
    }

    /// 追加一条摘要，**同时**声明它对应的结构化对象
    ///
    /// 这是 v0.26.0 之后写 handler 的**首选入口**：`push` 与结构化声明
    /// 一次调用完成，两者不会因为"改了其中一个忘了另一个"而漂移。
    ///
    /// 去重按 `(target_type, target_id, change_type)`：一次请求里
    /// 同一个对象被同一类变更反复 push 时只留一条 target，
    /// 而 `result` 文本照旧保留全部——文本是给人读的，重复叙述不算错。
    pub fn push_targeted(
        &self,
        line: impl Into<String>,
        target_type: TargetType,
        target_id: uuid::Uuid,
        change_type: ChangeType,
        target_label: Option<String>,
    ) {
        self.push_inner(
            line,
            Some(AuditTargetEntry::by_id(
                target_type,
                target_id,
                change_type,
                target_label.map(|l| truncate(&l, MAX_TARGET_LABEL_LEN)),
            )),
        );
    }

    fn push_inner(&self, line: impl Into<String>, target: Option<AuditTargetEntry>) {
        let line = line.into();
        // 空文本只跳过**文本**累加，不跳过 target：
        // `add_target` 传的正是空串，它要的就是"不要文本、只要结构化"。
        // 这里若连带 return 掉 target，`add_target` 就会静默变成空操作——
        // 而症状是"筛选查不到"，排查起来极难想到是这一行。
        if !line.trim().is_empty() {
            // 加锁失败（上一位持有者 panic）时静默丢弃这一条：
            // 审计摘要是旁路能力，为它 panic 等于让一个提示信息拖垮业务请求
            match self.lines.lock() {
                Ok(mut lines) => lines.push(line),
                Err(_) => tracing::warn!("审计摘要锁已中毒，丢弃本条摘要"),
            }
        }
        if let Some(t) = target {
            match self.targets.lock() {
                Ok(mut targets) => {
                    let dup = targets.iter().any(|e| {
                        e.target_type == t.target_type
                            && e.target_id == t.target_id
                            && e.target_key == t.target_key
                            && e.change_type == t.change_type
                    });
                    if !dup {
                        targets.push(t);
                    }
                }
                Err(_) => tracing::warn!("审计对象锁已中毒，丢弃本条结构化对象"),
            }
        }
    }

    /// **只**声明一个结构化对象，不追加任何摘要文本
    ///
    /// 用于"这次操作还牵涉到这些对象，但它们不值得各占一行摘要"的场景。
    /// 典型是角色授权：一次可能动几十个菜单，逐个写进 `result`
    /// 会把一条审计撑成长到没法看，而结构化查询要的只是
    /// "这些按钮被谁动过"，文本里那句 `权限码变更：授予 X、撤销 Y` 已经说清了。
    ///
    /// 没有它就只能用 `push_targeted` 硬塞，于是 `result` 里堆满
    /// `菜单权限码 "system:user:list"` 这类重复行——机器能筛了，人却读不了了。
    pub fn add_target(
        &self,
        target_type: TargetType,
        target_id: uuid::Uuid,
        change_type: ChangeType,
        target_label: Option<String>,
    ) {
        self.push_inner(
            String::new(),
            Some(AuditTargetEntry::by_id(
                target_type,
                target_id,
                change_type,
                target_label.map(|l| truncate(&l, MAX_TARGET_LABEL_LEN)),
            )),
        );
    }

    /// 以**字符串键**声明一个结构化对象，不追加摘要文本
    ///
    /// 给主键不是 UUID 的资源用，目前只有系统参数
    /// （`security.password.min_length` 这类参数名）。
    pub fn add_key_target(
        &self,
        target_type: TargetType,
        key: &str,
        change_type: ChangeType,
        target_label: Option<String>,
    ) {
        self.push_inner(
            String::new(),
            Some(AuditTargetEntry::by_key(
                target_type,
                truncate(key, MAX_TARGET_KEY_LEN),
                change_type,
                target_label.map(|l| truncate(&l, MAX_TARGET_LABEL_LEN)),
            )),
        );
    }

    /// 追加一条摘要并以字符串键声明其对象（系统参数用）
    pub fn push_key_targeted(
        &self,
        line: impl Into<String>,
        target_type: TargetType,
        key: &str,
        change_type: ChangeType,
        target_label: Option<String>,
    ) {
        self.push_inner(
            line,
            Some(AuditTargetEntry::by_key(
                target_type,
                truncate(key, MAX_TARGET_KEY_LEN),
                change_type,
                target_label.map(|l| truncate(&l, MAX_TARGET_LABEL_LEN)),
            )),
        );
    }

    /// 取出结构化对象引用；没有则返回空 `Vec`
    pub fn targets(&self) -> Vec<AuditTargetEntry> {
        match self.targets.lock() {
            Ok(targets) => targets.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// 合并成入库文本；没有任何摘要时返回 `None`
    pub fn render(&self) -> Option<String> {
        let joined = match self.lines.lock() {
            Ok(lines) => lines
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(DETAIL_SEPARATOR),
            Err(_) => return None,
        };
        if joined.is_empty() {
            None
        } else {
            Some(truncate(&joined, MAX_RESULT_LEN))
        }
    }
}

impl<S> FromRequestParts<S> for AuditDetail
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    /// 从请求扩展里取同一个累加器；取不到就返回一个**孤儿**实例
    ///
    /// 孤儿实例的 `push` 不会报错、也不会写进任何库——它的内容随请求结束
    /// 一起消失。这个方向是安全的：路由漏挂审计中间件时，
    /// 表现是"这一条没摘要"，而不是"请求失败"或"摘要串到别的请求上"。
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<AuditDetail>()
            .cloned()
            .unwrap_or_default())
    }
}

/// 操作日志中间件
pub async fn audit_log_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<impl IntoResponse, Response> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let params = req.uri().query().map(|q| truncate(q, MAX_QUERY_LEN));

    let auth_user = req.extensions().get::<AuthenticatedUser>().cloned();
    let client_ip = req
        .extensions()
        .get::<client_ip::ClientIp>()
        .map(|c| c.0.clone())
        .unwrap_or_else(|| client_ip::resolve_client_ip(&req, false));

    // 先挂累加器再进下游：handler 拿到的必须是**这一个** Arc，
    // 拿错了中间件就永远读到空摘要
    let detail = AuditDetail::default();
    req.extensions_mut().insert(detail.clone());

    let response = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as i32;
    let status_code = response.status().as_u16() as i32;

    // **只对 2xx 合并摘要**（理由见 [`summary_for`]）
    let result = summary_for(response.status().is_success(), &detail);
    // 结构化对象引用走**同一道门禁**：失败的请求不能声称它改过什么，
    // 文本与结构化必须一致——否则会出现"没有变更摘要、却查得到
    // 这个对象被改过"的矛盾记录，而矛盾记录比缺记录更难排查。
    let targets = if response.status().is_success() {
        detail.targets()
    } else {
        Vec::new()
    };

    let pool = state.auth_service.user_repo.pool().clone();

    // 异步写入，不阻塞响应
    tokio::spawn(async move {
        let (user_id, username) = match auth_user {
            Some(u) => (Some(u.user_id), Some(u.username)),
            None => (None, None),
        };
        // action 的列宽是 VARCHAR(512)（迁移 015）。这里**仍然截断**，
        // 因为写库失败只留一行 warn —— 而"某条路径的审计整条消失"
        // 是个不报错的失败：请求照常 200，事后却答不出"谁干过这件事"。
        // 截断至少留下一条可检索的记录。
        let action = truncate(&format!("{method} {path}"), MAX_ACTION_LEN);

        // 必须 `RETURNING id`：target 行要挂在这一条审计下面，
        // 而 id 是数据库生成的，插入前拿不到
        let result = sqlx::query_scalar::<_, uuid::Uuid>(
            r#"
            INSERT INTO audit_logs
                (user_id, username, action, method, path, params, result, status_code, client_ip, duration_ms)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(username)
        .bind(&action)
        .bind(&method)
        .bind(&path)
        .bind(params)
        .bind(result)
        .bind(status_code)
        .bind(&client_ip)
        .bind(duration_ms)
        .fetch_one(&pool)
        .await;

        let audit_log_id = match result {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("写入操作日志失败: {e}");
                return;
            }
        };

        for t in targets {
            // 单条 target 写失败**不放弃整条审计**：审计行已经落库了，
            // 丢掉它换一个"对象标注没写上"的缺口，代价大得多。
            // 这里只告警——和主写入路径同样的取舍。
            let res = sqlx::query(
                r#"
                INSERT INTO audit_log_targets
                    (audit_log_id, target_type, target_id, target_key, change_type, target_label)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(audit_log_id)
            .bind(t.target_type.as_str())
            .bind(t.target_id)
            .bind(t.target_key)
            .bind(t.change_type.as_str())
            .bind(t.target_label)
            .execute(&pool)
            .await;
            if let Err(e) = res {
                tracing::warn!("写入审计对象引用失败（{audit_log_id}）: {e}");
            }
        }
    });

    Ok(response)
}

/// 截断过长字符串，避免超长查询串撑大日志表
fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    value.chars().take(max_len).collect()
}

/// 决定这一次请求的摘要要不要进库
///
/// **为什么非成功响应必须丢弃摘要**：handler 追加摘要后仍可能失败
/// （写库成功、吊销会话失败等），此时把摘要写进去就是谎报
/// "已授予/已删除"。审计一旦开始说谎，比没有审计更危险——
/// 它会让人**不再去看**其他证据。
///
/// ## 这道门禁目前无法被集成测试触达（与直觉相反，如实记录）
///
/// 现有全部写 handler 都把 `audit.push` 放在**所有副作用成功之后**
/// （这是本版刻意的写法），因此"先 push 再失败"在接口层根本走不到——
/// 实测注入掉这道门禁后，`a_rejected_write_leaves_no_change_summary` 仍然全绿。
/// 也就是说集成测试证明的是"被拒的写操作不留摘要"，
/// 而它成立的原因**是 handler 的 push 时机，不是这道门禁**。
///
/// 门禁本身属于**纵深防御**：它防的是将来某个 handler 把 push 提前、
/// 或者某个中间件改写状态码。既然无法靠集成测试承重，
/// 就把它抽成纯函数用单测钉住两条分支，而不是让注释里的说法
/// 比实际验证到的更强。
fn summary_for(is_success: bool, detail: &AuditDetail) -> Option<String> {
    if is_success {
        detail.render()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_request_never_claims_it_changed_something() {
        let detail = AuditDetail::default();
        detail.push("角色 \"admin\" 已删除");
        assert!(detail.render().is_some(), "先确认累加器里确实有内容");
        // 门禁要拦住的正是这一条
        assert_eq!(summary_for(false, &detail), None);
    }

    #[test]
    fn a_successful_request_keeps_its_summary() {
        let detail = AuditDetail::default();
        detail.push("授予权限码 system:user:list");
        assert_eq!(
            summary_for(true, &detail).as_deref(),
            Some("授予权限码 system:user:list")
        );
    }

    #[test]
    fn blank_lines_are_dropped_and_none_is_kept() {
        let detail = AuditDetail::default();
        // 只读操作不该留下";"这种空壳
        assert_eq!(summary_for(true, &detail), None);
        detail.push("   ");
        detail.push("");
        assert_eq!(summary_for(true, &detail), None);
    }

    #[test]
    fn multiple_lines_join_into_one_readable_cell() {
        let detail = AuditDetail::default();
        detail.push("角色 \"admin\"（id）");
        detail.push("授予权限码 system:user:list");
        assert_eq!(
            summary_for(true, &detail).as_deref(),
            Some("角色 \"admin\"（id）；授予权限码 system:user:list")
        );
    }

    #[test]
    fn overlong_summaries_are_clipped_instead_of_growing_the_table_forever() {
        let detail = AuditDetail::default();
        detail.push("码".repeat(MAX_RESULT_LEN * 2));
        let rendered = summary_for(true, &detail).expect("摘要应存在");
        assert_eq!(rendered.chars().count(), MAX_RESULT_LEN);
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // 按字节截会把中文切成半个字，进库就是乱码
        assert_eq!(truncate("中文中文", 2), "中文");
        assert_eq!(truncate("ab", 5), "ab");
    }
}
