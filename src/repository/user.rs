//! 用户数据访问层（Repository）
//!
//! 封装对 `users` 表的所有数据库操作。
//! 使用 SQLx 异步连接池，通过 SQL 语句直接操作数据库（非 ORM 方式）。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::User;
use sqlx::PgPool;

/// `users` 表映射成 [`User`] 时必须**恰好**取到的列
///
/// `sqlx::FromRow` 要求结果集包含结构体的每一个字段，少一列就在**运行时**报错。
/// 本仓库原先有 8 处手写这份列名——迁移 010 加 `must_change_password` 时，
/// 忘了改 `controller/demo.rs` 那处，于是 `GET /api/admin/export/users`
/// 从 v0.11.0 起**每个调用都 500**，且没有任何测试碰过它（烂了七版）。
///
/// 与 `repository::menu::MENU_COLUMNS` 同理：集中成常量，加列只改这一处。
/// 守卫见 `tests/api_integration.rs` 的
/// `every_documented_endpoint_is_called_by_a_test`：新增 SELECT 手写列名会红。
pub const USER_COLUMNS: &str =
    "id, username, email, password_hash, is_active, must_change_password, display_name, avatar_url, created_at, updated_at";

/// 用户列表的筛选条件
///
/// 用一个结构体承载，而不是给 `list_filtered` 堆四个位置参数：
/// 后者每加一个维度就要改所有调用点，且极易把 `keyword` 和 `role_name` 传反位置
/// （两者都是 `&str` 形状，编译器抓不到）。
#[derive(Debug, Default, Clone)]
pub struct UserListFilter<'a> {
    /// 关键字，同时匹配用户名与邮箱
    pub keyword: Option<&'a str>,
    /// 激活状态；`None` 表示不限
    pub is_active: Option<bool>,
    /// 角色名（要求用户拥有该角色）；`None` 表示不限
    pub role_name: Option<&'a str>,
}

impl<'a> UserListFilter<'a> {
    /// 构造"不过滤"的空条件
    pub fn none() -> Self {
        Self::default()
    }

    /// 构造关键字筛选
    pub fn keyword(keyword: &'a str) -> Self {
        Self {
            keyword: Some(keyword),
            ..Default::default()
        }
    }

    /// 构造指定角色的筛选
    pub fn with_role(role_name: &'a str) -> Self {
        Self {
            role_name: Some(role_name),
            ..Default::default()
        }
    }

    /// 构造激活状态的筛选
    pub fn with_active(is_active: bool) -> Self {
        Self {
            is_active: Some(is_active),
            ..Default::default()
        }
    }

    /// 拼出 WHERE 片段与对应的绑定值
    ///
    /// 返回 `(条件片段, 参数)`，两者顺序严格对应：第 n 个 `$n`
    /// （WHERE 内的编号从 1 起）绑定 `params[n-1]`。
    pub fn build_where(&self) -> (Vec<String>, Vec<String>) {
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();

        // 空字符串/纯空白视为"不过滤"：搜索框清空后前端会发空串，
        // 若当成关键字就会筛出零条，看起来像"搜不到人"
        if let Some(k) = self.keyword.map(str::trim).filter(|k| !k.is_empty()) {
            params.push(format!(
                "%{}%",
                crate::utils::validation::escape_like_pattern(k)
            ));
            let idx = params.len();
            conditions.push(format!(
                r#"(username ILIKE ${idx} ESCAPE '\' OR email ILIKE ${idx} ESCAPE '\')"#
            ));
        }

        if let Some(active) = self.is_active {
            params.push(active.to_string());
            conditions.push(format!("is_active = ${}::boolean", params.len()));
        }

        if let Some(role) = self.role_name.map(str::trim).filter(|r| !r.is_empty()) {
            params.push(role.to_string());
            conditions.push(format!(
                r#"EXISTS (
                       SELECT 1 FROM user_roles ur
                       JOIN roles r ON r.id = ur.role_id
                       WHERE ur.user_id = users.id AND r.name = ${}
                   )"#,
                params.len()
            ));
        }

        (conditions, params)
    }
}

/// 把 `users` 上的唯一约束冲突翻译成 409，否则一律当内部错误
///
/// **每个字段有两个约束名，这是有意的**：迁移 001 的 `username VARCHAR(50) UNIQUE`
/// 叫 `users_username_key`，而迁移 013 为了大小写不敏感另建的
/// `CREATE UNIQUE INDEX ... ON users (lower(username))` 叫 `users_username_lower_key`。
/// 实测仅大小写不同的插入撞的是**后者**：
/// ```
/// INSERT INTO users (username, email, password_hash) VALUES ('ADMIN', 'x@e.com', 'h');
/// ERROR: duplicate key value violates unique constraint "users_username_lower_key"
/// ```
/// 只认旧名字的话，这条路径会落到 500 而不是 409——而它正是 013 注释里
/// 承诺的"第二道防线"，防线拦住了却报 500，等于把一个可诊断的冲突
/// 变成"用户说系统坏了"。
///
/// 两处调用点的措辞不同（`create` 说"已被注册"、`update` 说"已被占用"），
/// 那是**既有**的对外文案，不在这里顺手统一：改它对本次缺陷没有帮助，
/// 却会让前端/日志里已有的比对失效。措辞由调用方传入。
fn conflict_from(e: &sqlx::Error, action: &str, username_msg: &str, email_msg: &str) -> AppError {
    if let Some(pg_err) = e.as_database_error() {
        if let Some(constraint) = pg_err.constraint() {
            if constraint == "users_username_key" || constraint == "users_username_lower_key" {
                return AppError::Conflict(username_msg.to_string());
            }
            if constraint == "users_email_key" || constraint == "users_email_lower_key" {
                return AppError::Conflict(email_msg.to_string());
            }
        }
    }
    AppError::InternalServerError(format!("{action}: {e}"))
}

/// 用户仓储
///
/// 提供用户相关的数据库 CRUD 操作。
#[derive(Debug, Clone)]
pub struct UserRepository {
    /// SQLx PostgreSQL 连接池
    pub pool: PgPool,
}

impl UserRepository {
    /// 创建新的 UserRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 数据库连接池访问器
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 根据用户 ID 查找用户
    ///
    /// # Arguments
    ///
    /// * `id` - 用户 UUID
    ///
    /// # Returns
    ///
    /// 找到返回 `User`，未找到返回 `AppError::NotFound`。
    pub async fn find_by_id(&self, id: Uuid) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users
            WHERE id = $1
            "#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
    }

    /// 根据用户名查找用户（精确匹配）
    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users
            WHERE username = $1
            "#
        ))
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 根据用户名或邮箱查找用户（用于登录）
    ///
    /// 支持用户名或邮箱两种方式的登录查询。
    pub async fn find_by_username_or_email(&self, input: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users
            WHERE username = $1 OR email = $1
            "#
        ))
        .bind(input)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 根据邮箱查找用户
    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users
            WHERE email = $1
            "#
        ))
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))
    }

    /// 分页查询用户列表（可组合筛选）
    ///
    /// **筛选维度**：关键字（同时匹配用户名与邮箱）、激活状态、指定角色。
    ///
    /// 关键字同时匹配 `username` 与 `email`。**匹配前必须转义**（见
    /// [`crate::utils::validation::escape_like_pattern`]），否则搜 `100%`
    /// 会因 `%` 是通配符而返回全表——一个"筛选"比不筛选还糟。
    ///
    /// ── 为什么改成动态拼 WHERE 而不是 `match` 穷举分支 ──
    /// 原实现按 `keyword` 有无分两条完整 SQL。加两个筛选维度后要 8 个分支，
    /// 而每条分支里的 WHERE 与 COUNT 必须**逐字一致**，否则就是
    /// "列表 3 条但 total 500"的分页错乱。分支越多，漏改一处越容易，
    /// 而且编译器不会提醒。
    ///
    /// 现在条件与参数都按**同一个顺序**推进，列表与计数复用同一段拼接逻辑，
    /// 结构上就不可能对不上。
    ///
    /// ── `role_name` 用 EXISTS 子查询而不是 JOIN ──
    /// 一个用户可能同时命中多条角色行（多对多），JOIN 会让同一用户重复出现
    /// 并把 total 算大。`EXISTS` 只问"有没有"，天然不放大行数。
    ///
    /// `role_name` 与 `is_active` 一起给时是 **AND**：两个条件都要满足。
    /// 这不是实现偷懒——"既是 HR 又是禁用的"是一个明确的问法，
    /// 若改成 OR（满足其一即命中）会返回一批用户没预期的账号。
    pub async fn list_filtered(
        &self,
        page: i64,
        page_size: i64,
        filter: &UserListFilter<'_>,
    ) -> Result<(Vec<User>, i64), AppError> {
        let offset = (page - 1) * page_size;
        let (where_sql, params) = filter.build_where();

        // 没有筛选条件时不拼 WHERE 子句，而不是拼一个恒真的条件：
        // 后者会让"有没有过滤"这件事从 SQL 文本上就看不出来，读代码的人得反推。
        let where_clause = if params.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_sql.join(" AND "))
        };

        let list_sql = format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users{where_clause}
            ORDER BY created_at DESC
            LIMIT ${} OFFSET ${}
            "#,
            params.len() + 1,
            params.len() + 2
        );

        let count_sql = format!("SELECT COUNT(*) FROM users{where_clause}");

        // 一次构建两种查询共用的一组绑定值。
        // QueryBuilder 那类 API 在这里反而更绕：参数类型随条件数量变化，
        // 动态拼接 + bind 反而能把"SQL 里的 $n 与 binds 的顺序"写成一段线性代码，
        // 读起来就知道它们不会错位。
        let mut list_stmt = sqlx::query_as::<_, User>(&list_sql);
        let mut count_stmt = sqlx::query_as::<_, (i64,)>(&count_sql);
        for p in &params {
            list_stmt = list_stmt.bind(p);
            count_stmt = count_stmt.bind(p);
        }
        let users = list_stmt
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询用户列表失败: {e}")))?;

        let total: (i64,) = count_stmt
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询用户总数失败: {e}")))?;

        Ok((users, total.0))
    }

    /// 更新用户信息（用户名、邮箱、是否激活）
    pub async fn update(
        &self,
        id: Uuid,
        username: &str,
        email: &str,
        is_active: bool,
    ) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            UPDATE users
            SET username = $2, email = $3, is_active = $4
            WHERE id = $1
            RETURNING {USER_COLUMNS}
            "#
        ))
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(is_active)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(target: "repository", "更新用户失败 (id={}): {:?}", id, e);
            conflict_from(&e, "更新用户失败", "用户名已被占用", "邮箱已被占用")
        })
    }

    /// 自助修改资料（仅展示型字段）
    ///
    /// 与管理员的 [`Self::update`] **刻意分开**，不能合并成一个方法加个开关：
    /// 管理员那条会写 `username` / `email` / `is_active`，而这条一个都不碰。
    /// 合成"可选参数版"的后果是调用方多传一个字段就静默生效——
    /// 让自助端点具备了改登录键的能力，正是 v0.19.0 刚修掉的那类"意外可达"。
    ///
    /// ── 为什么**不能**用 `COALESCE($3, display_name)` ──
    /// 最初就是这么写的，结果"清空展示名"永远不生效：
    /// `COALESCE(NULL, display_name)` 的语义是"传了 NULL 就保留旧值"，
    /// 而请求里"显式清空"正是要传 NULL。两个不同的意思被压进了同一个 COALESCE。
    /// 集成测试 `a_user_can_set_and_clear_their_own_display_name` 当场抓到了。
    ///
    /// 正确写法是让**外层 flag 决定写不写**，内层值原样绑定——
    /// 内层是 NULL 就写 NULL，那才是"清空"。
    ///
    /// 三态语义（外层 / 内层 → 行为）：
    /// - `None` / —     → 不碰该列
    /// - `Some(None)`   → 写 NULL（清空）
    /// - `Some(Some(v))`→ 写 v
    ///
    /// 用外层 flag 而不是 COALESCE，还让前端不必先读出旧值再原样写回——
    /// 回写一个刚刚被别人改过的旧值就是典型的丢失更新。
    pub async fn update_profile(
        &self,
        id: Uuid,
        display_name: Option<Option<&str>>,
        avatar_url: Option<Option<&str>>,
    ) -> Result<User, AppError> {
        // 两个都没给就是"什么都不改"。仍然走一次 UPDATE 让不存在的 id
        // 报出可诊断的错误，而不是静默返回成功——调用方需要知道这次是否真的生效。
        let user = sqlx::query_as::<_, User>(&format!(
            r#"
            UPDATE users
            SET display_name = CASE WHEN $2::boolean THEN $3::text ELSE display_name END,
                avatar_url   = CASE WHEN $4::boolean THEN $5::text ELSE avatar_url   END
            WHERE id = $1
            RETURNING {USER_COLUMNS}
            "#
        ))
        .bind(id)
        .bind(display_name.is_some())
        .bind(display_name.flatten().map(str::to_string))
        .bind(avatar_url.is_some())
        .bind(avatar_url.flatten().map(str::to_string))
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(target: "repository", "更新用户资料失败 (id={}): {:?}", id, e);
            AppError::InternalServerError(format!("更新用户资料失败: {e}"))
        })?;

        match user {
            Some(u) => Ok(u),
            // 不把"用户不存在"报成 404：这个端点用当前登录者的 id，
            // 拿不到用户说明账号刚被删或会话已失效，由上层统一处理
            None => Err(AppError::NotFound("用户不存在".into())),
        }
    }

    /// 批量查询多个用户的角色（避免列表页 N+1）
    ///
    /// 返回 `(user_id, role_name)`，调用方按 user_id 归组。
    pub async fn find_roles_for_users(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>, AppError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        sqlx::query_as::<_, (Uuid, String)>(
            r#"
            SELECT ur.user_id, r.name
            FROM user_roles ur
            JOIN roles r ON r.id = ur.role_id
            WHERE ur.user_id = ANY($1)
            ORDER BY r.name ASC
            "#,
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户角色失败: {e}")))
    }

    /// 更新密码哈希（用于改密与 v0.1 旧口令格式升级）
    pub async fn update_password_hash(
        &self,
        id: Uuid,
        password_hash: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("更新密码失败: {e}")))?;
        Ok(())
    }

    /// 删除用户
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除用户失败: {e}")))?;
        Ok(())
    }

    /// 创建新用户
    ///
    /// # Arguments
    ///
    /// * `id` - 新用户 UUID
    /// * `username` - 用户名
    /// * `email` - 电子邮箱
    /// * `password_hash` - Argon2 哈希后的密码
    ///
    /// # Returns
    ///
    /// 返回创建成功的用户记录。
    /// 创建新用户
    ///
    /// # Arguments
    ///
    /// * `id` - 新用户 UUID
    /// * `username` - 用户名
    /// * `email` - 电子邮箱
    /// * `password_hash` - Argon2 哈希后的密码
    /// * `must_change_password` - 是否要求首次登录后立即改密。
    ///   **管理员建号应传 `true`**（口令是管理员定的，用户本人没参与）；
    ///   公开注册应传 `false`（用户自己设的口令）
    ///
    /// # Returns
    ///
    /// 返回创建成功的用户记录。
    pub async fn create(
        &self,
        id: Uuid,
        username: &str,
        email: &str,
        password_hash: &str,
        must_change_password: bool,
    ) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(&format!(
            r#"
            INSERT INTO users (id, username, email, password_hash, is_active, must_change_password)
            VALUES ($1, $2, $3, $4, true, $5)
            RETURNING {USER_COLUMNS}
            "#
        ))
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .bind(must_change_password)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // 上层的查重已经挡掉了绝大多数重复，这条只兜"两个并发请求同时通过查重"
            // 那种窗口期——所以它报的是唯一约束，不是业务判定
            conflict_from(&e, "创建用户失败", "用户名已被注册", "邮箱已被注册")
        })
    }

    /// 置位"必须改密"标记
    ///
    /// 管理员重置口令后调用：重置意味着用户**本人没参与**这次口令选择，
    /// 因此必须让其在下次登录后改掉。
    pub async fn set_must_change_password(&self, id: Uuid, required: bool) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET must_change_password = $2 WHERE id = $1")
            .bind(id)
            .bind(required)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("更新强制改密标记失败: {e}")))?;
        Ok(())
    }
}
