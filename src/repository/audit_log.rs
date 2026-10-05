//! 操作日志数据访问层
//!
//! 读取 + 写入。
//!
//! **两条写入路径的语义不同，不要混用**：
//! - `middleware::audit_log`：`tokio::spawn` 异步写、不阻塞响应、失败只告警。
//!   用于常规受保护路由——丢一条日志不该让用户的业务请求失败
//! - [`AuditLogRepository::record`]：**同步 `await`**、失败即返回 `Err`。
//!   用于登录/注册这类**安全关键**路径：审计写不进去就必须让请求失败，
//!   否则"审计"又变成一个失败时无声的能力

use uuid::Uuid;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder};

use crate::error::AppError;
use crate::model::{
    AuditLog, AuditLogTarget, AuditLogTargetRow, AuditLogWithTargets, AuditTargetEntry,
};
use crate::utils::pagination::{PaginatedResponse, PaginationParams};

/// 一条待写入的审计记录
///
/// 与 [`AuditLog`]（读出行）分开：写入侧的身份字段绝大多数可空，
/// 尤其是 `user_id`/`username`——**登录失败时可能根本没有对应用户**。
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// 已认证用户 ID；未认证场景（如登录失败）为 `None`
    pub user_id: Option<Uuid>,
    /// 用户名；登录失败时记**尝试登录的名字**，而不是查无此人
    pub username: Option<String>,
    /// 动作标识
    pub action: String,
    /// HTTP 方法
    pub method: String,
    /// 请求路径
    pub path: String,
    /// 查询串
    pub params: Option<String>,
    /// 结果说明（登录审计用它写失败原因）
    pub result: Option<String>,
    /// HTTP 状态码
    pub status_code: Option<i32>,
    /// 客户端 IP
    pub client_ip: Option<String>,
    /// 耗时（毫秒）
    pub duration_ms: Option<i32>,
    /// 结构化对象引用（v0.26.0）
    pub targets: Vec<AuditTargetEntry>,
}

impl AuditEntry {
    /// 登录 / 注册审计的构造入口
    ///
    /// `action` 用**语义值**（`AUTH_LOGIN_SUCCESS` 等）而不是 `{METHOD} {path}`：
    /// 登录成功与失败的方法、路径**完全相同**，只有 action 与身份有区分度，
    /// 用方法+路径就无法回答"有没有人在爆破"。
    pub fn auth(action: &str, method: &str, path: &str, status_code: i32, ip: &str) -> Self {
        Self {
            user_id: None,
            username: None,
            action: action.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            params: None,
            result: None,
            status_code: Some(status_code),
            client_ip: Some(ip.to_string()),
            duration_ms: None,
            targets: Vec::new(),
        }
    }

    /// 附上一个结构化对象引用
    ///
    /// 登录/注册这类语义 action 也要能回答"这次登录动的是哪个账号"——
    /// 它不走审计中间件，因此只能在这里声明。
    pub fn with_target(
        mut self,
        target_type: crate::model::TargetType,
        target_id: Uuid,
        change_type: crate::model::ChangeType,
        target_label: Option<String>,
    ) -> Self {
        self.targets.push(AuditTargetEntry::by_id(
            target_type,
            target_id,
            change_type,
            target_label,
        ));
        self
    }

    /// 附上身份信息
    pub fn with_identity(mut self, user_id: Option<Uuid>, username: Option<String>) -> Self {
        self.user_id = user_id;
        self.username = username;
        self
    }

    /// 附上结果说明
    pub fn with_result(mut self, result: impl Into<String>) -> Self {
        self.result = Some(result.into());
        self
    }
}

/// 插入一条审计主行，返回它的 id
///
/// 抽成自由函数是为了让 [`AuditLogRepository::record`] 与
/// `middleware::audit_log` 复用**同一条 INSERT**。
/// 两条路径以前各写一份，字段一旦不同步就会出现
/// "某些审计行少记了某个字段"这种只在部分入口复现的偏差。
///
/// 泛型 `E` 同时接受 `&PgPool` 与 `&mut Transaction`：
/// 主行既可能单独落库（target 写失败后的补写），
/// 也可能与 targets 同处一个事务（正常路径）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_audit_row<'e, E>(
    executor: E,
    user_id: Option<Uuid>,
    username: Option<&str>,
    action: &str,
    method: &str,
    path: &str,
    params: Option<&str>,
    result: Option<&str>,
    status_code: Option<i32>,
    client_ip: Option<&str>,
    duration_ms: Option<i32>,
) -> Result<Uuid, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO audit_logs
            (user_id, username, action, method, path, params, result, status_code, client_ip, duration_ms)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(username)
    .bind(action)
    .bind(method)
    .bind(path)
    .bind(params)
    .bind(result)
    .bind(status_code)
    .bind(client_ip)
    .bind(duration_ms)
    .fetch_one(executor)
    .await
}

/// 插入一条审计对象引用
///
/// 与 [`insert_audit_row`] 同理，两条写入路径共用一份 SQL。
/// `clip` 由调用方传入而不是在这里重建：两条路径的截断上限
/// 本来就一致，但共用同一个闭包能保证**永远**一致。
pub(crate) async fn insert_audit_target<'e, E, F>(
    executor: E,
    audit_log_id: Uuid,
    t: &AuditTargetEntry,
    clip: &F,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Postgres>,
    // 必须 `Send + Sync`：调用方之一在 `tokio::spawn` 里，
    // 用裸 `&dyn Fn` 会让整个 future 变成非 Send，
    // 报错还牵连出一串与本改动无关的 Handler 约束错误。
    F: Fn(&str, usize) -> String + Send + Sync,
{
    sqlx::query(
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
    .bind(t.target_key.as_deref().map(|k| clip(k, 200)))
    .bind(t.change_type.as_str())
    .bind(t.target_label.as_deref().map(|l| clip(l, 200)))
    .execute(executor)
    .await
    .map(|_| ())
}

/// 操作日志仓储
#[derive(Debug, Clone)]
pub struct AuditLogRepository {
    pool: PgPool,
}

/// 审计日志筛选条件
///
/// 空串与纯空白一律视为"不过滤"：筛选框清空后前端会发空串，
/// 若当成条件就会筛出零条，看起来像"日志没了"。
#[derive(Debug, Clone, Default)]
pub struct AuditLogFilter {
    pub username: Option<String>,
    pub action: Option<String>,
    pub status_code: Option<i32>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    /// 对象种类，如 `role`。**与 `target_id` 配对使用**
    pub target_type: Option<String>,
    /// 被操作对象 ID。单独给 `target_id` 而不给 `target_type` 是**允许**的，
    /// 语义是"这个对象被谁动过"，不限定它是哪一类
    pub target_id: Option<Uuid>,
    /// 字符串主键的对象（系统参数名）。与 `target_id` 二选一
    pub target_key: Option<String>,
}

impl AuditLogFilter {
    /// 全为空即"无筛选"。导出用它决定要不要保留行数上限。
    pub fn is_empty(&self) -> bool {
        self.username.is_none()
            && self.action.is_none()
            && self.status_code.is_none()
            && self.start_time.is_none()
            && self.end_time.is_none()
            && self.target_type.is_none()
            && self.target_id.is_none()
            && self.target_key.is_none()
    }
}

fn norm(v: Option<&String>) -> Option<String> {
    v.map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 把筛选条件拼进 WHERE 子句
///
/// **用 `QueryBuilder` 动态拼接，而不是 `($1::text IS NULL OR ...)` 那套恒真写法**：
/// 后者虽然省事，但 Postgres 看见的是"对每列都可能为真"的 OR，
/// 优化器无法把它化成索引扫描——日志表越大越慢。
/// 这里每个条件缺失时**根本不进 SQL**，索引照常可用。
///
/// 片段本身是常量，用户输入一律走绑定参数，不存在拼接用户输入的情况。
fn push_filters<'a>(qb: &mut QueryBuilder<'a, Postgres>, f: &'a AuditLogFilter) {
    if let Some(u) = norm(f.username.as_ref()) {
        qb.push(" AND username ILIKE ")
            .push_bind(format!(
                "%{}%",
                crate::utils::validation::escape_like_pattern(&u)
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(a) = norm(f.action.as_ref()) {
        qb.push(" AND action ILIKE ")
            .push_bind(format!(
                "%{}%",
                crate::utils::validation::escape_like_pattern(&a)
            ))
            .push(" ESCAPE '\\'");
    }
    if let Some(code) = f.status_code {
        qb.push(" AND status_code = ").push_bind(code);
    }
    if let Some(t) = f.start_time {
        qb.push(" AND created_at >= ").push_bind(t);
    }
    if let Some(t) = f.end_time {
        qb.push(" AND created_at <= ").push_bind(t);
    }
    // target 条件走 `EXISTS` 子查询而不是 JOIN：
    // JOIN 会让一条审计行出现 N 次（N 个 target），
    // 于是 COUNT 与列表的 total 对不上、分页还会漏行。
    // EXISTS 只回答"有没有"，天然不放大行数。
    if let Some(tt) = norm(f.target_type.as_ref()) {
        qb.push(
            " AND EXISTS (SELECT 1 FROM audit_log_targets t \
             WHERE t.audit_log_id = audit_logs.id AND t.target_type = ",
        )
        .push_bind(tt)
        .push(")");
    }
    if let Some(tid) = f.target_id {
        qb.push(
            " AND EXISTS (SELECT 1 FROM audit_log_targets t \
             WHERE t.audit_log_id = audit_logs.id AND t.target_id = ",
        )
        .push_bind(tid)
        .push(")");
    }
    if let Some(tk) = norm(f.target_key.as_ref()) {
        qb.push(
            " AND EXISTS (SELECT 1 FROM audit_log_targets t \
             WHERE t.audit_log_id = audit_logs.id AND t.target_key = ",
        )
        .push_bind(tk)
        .push(")");
    }
}

const SELECT_COLS: &str = "id, user_id, username, action, method, path, params, result, \
     status_code, client_ip, duration_ms, created_at";

impl AuditLogRepository {
    /// 创建新的 AuditLogRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 同步写入一条审计记录
    ///
    /// 与中间件的 `tokio::spawn` 路径相反：**写不进去就返回 `Err`**。
    /// 登录/注册是安全关键路径，审计静默丢失等于没审计。
    ///
    /// **为什么这里必须截断而不能靠数据库报错**：`username` 是登录时的
    /// **用户输入**，列宽只有 `VARCHAR(50)`。不截断的话，一个 200 字符的
    /// 用户名会让 INSERT 报 `value too long`，于是"口令错误"被升级成 500 ——
    /// 审计反而成了拒绝服务的入口。失败信息也必须能落库，不能反过来打挂登录。
    pub async fn record(&self, entry: &AuditEntry) -> Result<(), AppError> {
        let clip = |v: &str, max: usize| -> String { v.chars().take(max).collect::<String>() };

        // 主行与它的 targets 必须在**同一个事务**里落库（v0.28.0 补）
        //
        // 曾经是"主行先落、targets 再逐条补"，两步之间没有事务，
        // 于是读方可能在 targets 写完之前就看到这一行。
        // 表现是"一次授权碰了 3 个菜单，审计里只挂着 1~2 个 target"——
        // 而"一次请求碰了 N 个对象不能只记第一个"恰恰是 v0.26.0
        // 结构化审计要解决的核心问题。
        //
        // 这个缺陷在本地跑不出来：写入窗口只有几毫秒，
        // 而轮询间隔是 100ms，大概率整个写入早已完成。
        // CI 上稳定复现（两个审计 target 用例红），也是这么暴露的。
        let username = entry.username.as_deref().map(|u| clip(u, 50));
        let action = clip(&entry.action, 100);
        let method = clip(&entry.method, 10);
        let path = clip(&entry.path, 500);
        let client_ip = entry.client_ip.as_deref().map(|ip| clip(ip, 50));

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("审计事务开启失败: {e}")))?;

        let audit_log_id = insert_audit_row(
            &mut *tx,
            entry.user_id,
            username.as_deref(),
            &action,
            &method,
            &path,
            entry.params.as_deref(),
            entry.result.as_deref(),
            entry.status_code,
            client_ip.as_deref(),
            entry.duration_ms,
        )
        .await
        .map_err(|e| AppError::InternalServerError(format!("写入审计日志失败: {e}")))?;

        for t in &entry.targets {
            if let Err(e) = insert_audit_target(&mut *tx, audit_log_id, t, &clip).await {
                // 回滚后**单独**补写主行再报错，理由见函数末尾的说明：
                // "审计行丢失"比"对象标注没写上"代价大得多。
                tracing::warn!("写入审计对象引用失败，回滚后补写主行: {e}");
                let _ = tx.rollback().await;
                insert_audit_row(
                    &self.pool,
                    entry.user_id,
                    username.as_deref(),
                    &action,
                    &method,
                    &path,
                    entry.params.as_deref(),
                    entry.result.as_deref(),
                    entry.status_code,
                    client_ip.as_deref(),
                    entry.duration_ms,
                )
                .await
                .map_err(|e2| AppError::InternalServerError(format!("写入审计日志失败: {e2}")))?;
                return Err(AppError::InternalServerError(format!(
                    "写入审计对象引用失败: {e}"
                )));
            }
        }

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("审计事务提交失败: {e}")))?;
        Ok(())
    }

    /// 分页查询操作日志（可按条件筛选）
    ///
    /// 表名与排序列均为编译期常量，不存在拼接用户输入的情况；
    /// 筛选值一律走绑定参数。
    ///
    /// 计数与取数**共用同一组筛选条件**——两者一旦不一致，
    /// 就会出现"列表 3 条但 total 500"的分页错乱，
    /// 那比筛选失效更容易让人误判数据规模。
    pub async fn paginate(
        &self,
        params: &PaginationParams,
        filter: &AuditLogFilter,
    ) -> Result<PaginatedResponse<AuditLogWithTargets>, AppError> {
        const ALLOWED_SORT_FIELDS: [&str; 4] = ["created_at", "username", "action", "status_code"];

        let page = params.get_page();
        let page_size = params.get_page_size();
        let offset = params.get_offset();
        let order_sql = params.get_order_sql(&ALLOWED_SORT_FIELDS);

        let mut count_qb: QueryBuilder<Postgres> =
            QueryBuilder::new("SELECT COUNT(*) FROM audit_logs WHERE TRUE");
        push_filters(&mut count_qb, filter);
        let total: (i64,) = count_qb
            .build_query_as()
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志总数失败: {e}")))?;

        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT {SELECT_COLS} FROM audit_logs WHERE TRUE"));
        push_filters(&mut qb, filter);
        // `order_sql` 来自字段白名单，不含用户输入
        qb.push(format!(" ORDER BY {order_sql} LIMIT "))
            .push_bind(page_size)
            .push(" OFFSET ")
            .push_bind(offset);

        let items = qb
            .build_query_as::<AuditLog>()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

        let targets = self.targets_for(&items).await?;

        Ok(PaginatedResponse::new(
            items
                .into_iter()
                .map(|log| {
                    let mut row = AuditLogWithTargets::from(log.clone());
                    row.targets = targets.get(&log.id).cloned().unwrap_or_default();
                    row
                })
                .collect(),
            total.0,
            page,
            page_size,
        ))
    }

    /// 批量取一批审计行的结构化对象引用，返回 `audit_log_id -> targets`
    ///
    /// **一次查询而不是每行一次**：列表页一页 20 条，逐行查就是 20 次往返。
    /// 空列表直接返回空 map，不发查询——`WHERE id = ANY('{}')` 虽合法但无意义。
    /// 批量取一批日志的 target，返回 `audit_log_id → targets`
    ///
    /// **对外公开**是因为导出处点也要用：Excel 里少一列「涉及对象」，
    /// 就等于结构化数据只在一半的读路径上存在——和 v0.13.0
    /// 「`result` 只在库里、界面与导出都读不到」是同一类失效。
    pub async fn targets_for(
        &self,
        logs: &[AuditLog],
    ) -> Result<std::collections::HashMap<Uuid, Vec<AuditLogTarget>>, AppError> {
        if logs.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let ids = logs.iter().map(|l| l.id).collect::<Vec<_>>();
        let rows = sqlx::query_as::<_, AuditLogTargetRow>(
            "SELECT audit_log_id, target_type, target_id, target_key, change_type, target_label \
             FROM audit_log_targets WHERE audit_log_id = ANY($1) \
             ORDER BY target_type, COALESCE(target_key, target_id::text)",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询审计对象引用失败: {e}")))?;

        let mut map: std::collections::HashMap<Uuid, Vec<AuditLogTarget>> =
            std::collections::HashMap::new();
        for row in rows {
            map.entry(row.audit_log_id).or_default().push(row.target);
        }
        Ok(map)
    }

    /// 按筛选条件取日志用于导出，返回 `(导出的行, 是否被上限截断)`
    ///
    /// **上限必须连同"是否截断"一起返回**。原实现硬编码 `LIMIT 10000`
    /// 且不告诉任何人，于是用户以为导出了全量，实际只有最新一万条——
    /// 静默截断比报错更糟：报错至少让人知道要缩小范围。
    pub async fn fetch_for_export(
        &self,
        filter: &AuditLogFilter,
        max_rows: i64,
    ) -> Result<(Vec<AuditLog>, bool), AppError> {
        // 多取一行用来判断是否触顶：若恰好取回 max_rows + 1，
        // 说明还有更多数据被砍掉了
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new(format!("SELECT {SELECT_COLS} FROM audit_logs WHERE TRUE"));
        push_filters(&mut qb, filter);
        qb.push(" ORDER BY created_at DESC LIMIT ")
            .push_bind(max_rows + 1);

        let mut rows: Vec<AuditLog> = qb
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

        let truncated = rows.len() as i64 > max_rows;
        if truncated {
            rows.truncate(max_rows as usize);
        }
        Ok((rows, truncated))
    }

    /// 删除 `cutoff` 之前的日志，返回删除行数与是否因上限提前收手
    ///
    /// **分批**删除，而不是一条 `DELETE FROM audit_logs WHERE created_at < $1`：
    /// 一次性删几十万行会长时间持锁并把 WAL 撑爆，期间其他事务只能干等。
    /// 分批把锁持有时间切碎；某批没删满即说明已删到 cutoff 附近，提前收工。
    ///
    /// 子查询按 `created_at` 升序取最旧的一批，
    /// 正好反向扫描 `idx_audit_logs_created`，不必全表排序。
    ///
    /// `hit_batch_limit` 区分"清干净了"与"撞上限收手"：撞上限时
    /// `cutoff_at` **之后**可能还有过期行留在库里。不报这个区别，
    /// 就会把"还有更多过期数据没清"当成"已经清干净了"，
    /// 而这正是保留策略最需要如实告知的那件事。
    pub async fn delete_older_than(
        &self,
        cutoff: DateTime<Utc>,
        batch_size: i64,
        max_batches: u32,
    ) -> Result<PurgeOutcome, AppError> {
        let batch_size = batch_size.max(1);
        let mut total_deleted: u64 = 0;
        let mut hit_batch_limit = false;

        for _ in 0..max_batches {
            let deleted = sqlx::query(
                r#"
                DELETE FROM audit_logs WHERE id IN (
                    SELECT id FROM audit_logs
                    WHERE created_at < $1
                    ORDER BY created_at
                    LIMIT $2
                )
                "#,
            )
            .bind(cutoff)
            .bind(batch_size)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清理过期操作日志失败: {e}")))?
            .rows_affected();

            total_deleted += deleted;
            if deleted < batch_size as u64 {
                // 没删满说明已删到 cutoff 附近，属于清干净了
                break;
            }
            // 每一批都恰好删满，且循环还有下一轮 —— 说明还有过期行没轮到
            hit_batch_limit = true;
        }

        Ok(PurgeOutcome {
            deleted: total_deleted,
            hit_batch_limit,
        })
    }

    /// 把一轮清理记进 `audit_log_purges`，让"日志被清掉了"这件事本身可查
    pub async fn record_purge(
        &self,
        cutoff_at: DateTime<Utc>,
        outcome: &PurgeOutcome,
        duration_ms: i32,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO audit_log_purges
                (cutoff_at, deleted_rows, duration_ms, hit_batch_limit)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(cutoff_at)
        .bind(outcome.deleted as i64)
        .bind(duration_ms)
        .bind(outcome.hit_batch_limit)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("记录审计清理动作失败: {e}")))?;
        Ok(())
    }

    /// 最近一次清理记录（`None` 表示启用保留策略后一次都还没删过）
    pub async fn latest_purge(&self) -> Result<Option<AuditLogPurge>, AppError> {
        let row = sqlx::query_as::<_, (DateTime<Utc>, i64, DateTime<Utc>, Option<i32>, bool)>(
            r#"
            SELECT cutoff_at, deleted_rows, ran_at, duration_ms, hit_batch_limit
            FROM audit_log_purges
            ORDER BY ran_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询最近清理记录失败: {e}")))?;

        Ok(row.map(
            |(cutoff_at, deleted_rows, ran_at, duration_ms, hit_batch_limit)| AuditLogPurge {
                cutoff_at,
                deleted_rows,
                ran_at,
                duration_ms,
                hit_batch_limit,
            },
        ))
    }

    /// 现存日志里最老一条的时刻
    ///
    /// 这是"还能查到多早的数据"的真实答案。返回 `None` 表示表是空的——
    /// 此时界面不能说"数据早到 X"，也不能说"数据都是最新的"，
    /// 只能如实显示"暂无日志"。
    pub async fn oldest_log_at(&self) -> Result<Option<DateTime<Utc>>, AppError> {
        // `query_scalar` 而非 `query_as`：单值的可空结果用标量读更直白
        let oldest: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT MIN(created_at) FROM audit_logs")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::InternalServerError(format!("查询最旧日志时刻失败: {e}")))?;
        Ok(oldest)
    }
}

/// 一轮清理的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeOutcome {
    /// 实际删除行数
    pub deleted: u64,
    /// 是否因达到单轮批数上限而提前收手
    pub hit_batch_limit: bool,
}

/// `audit_log_purges` 的读出行
#[derive(Debug, Clone)]
pub struct AuditLogPurge {
    /// 本轮删掉的行都早于该时刻
    pub cutoff_at: DateTime<Utc>,
    /// 删掉的行数
    pub deleted_rows: i64,
    /// 本轮执行时刻
    pub ran_at: DateTime<Utc>,
    /// 耗时（毫秒）
    pub duration_ms: Option<i32>,
    /// 是否因上限提前收手
    pub hit_batch_limit: bool,
}
