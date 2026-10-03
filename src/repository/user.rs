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
    "id, username, email, password_hash, is_active, must_change_password, created_at, updated_at";

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

    /// 查询所有用户（分页）
    /// 用户列表（可按关键字过滤）
    ///
    /// `keyword` 为 `None` 时行为与原先的 `list_all` 完全一致——不拼任何
    /// `WHERE` 片段，而不是拼一个恒真的条件：后者会让"有没有过滤"这件事
    /// 从 SQL 文本上就看不出来，读代码的人得反推。
    ///
    /// 关键字同时匹配 `username` 与 `email`。**匹配前必须转义**（见
    /// [`crate::utils::validation::escape_like_pattern`]），否则搜 `100%`
    /// 会因 `%` 是通配符而返回全表——一个"筛选"比不筛选还糟。
    pub async fn list_filtered(
        &self,
        page: i64,
        page_size: i64,
        keyword: Option<&str>,
    ) -> Result<(Vec<User>, i64), AppError> {
        let offset = (page - 1) * page_size;
        // 空字符串/纯空白视为"不过滤"：搜索框清空后前端会发空串，
        // 若当成关键字就会筛出零条，看起来像"搜不到人"
        let keyword = keyword.map(str::trim).filter(|k| !k.is_empty());

        let users = match keyword {
            None => {
                sqlx::query_as::<_, User>(&format!(
                    r#"
                    SELECT {USER_COLUMNS}
                    FROM users
                    ORDER BY created_at DESC
                    LIMIT $1 OFFSET $2
                    "#
                ))
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
            }
            Some(k) => {
                let pattern = format!("%{}%", crate::utils::validation::escape_like_pattern(k));
                sqlx::query_as::<_, User>(&format!(
                    r#"
                    SELECT {USER_COLUMNS}
                    FROM users
                    WHERE username ILIKE $1 ESCAPE '\' OR email ILIKE $1 ESCAPE '\'
                    ORDER BY created_at DESC
                    LIMIT $2 OFFSET $3
                    "#
                ))
                .bind(&pattern)
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| AppError::InternalServerError(format!("查询用户列表失败: {e}")))?;

        // 计数必须用**同一个** WHERE，否则会出现"列表 3 条但 total 500"
        // 的分页错乱——那比筛选失效更容易让人误判数据规模
        let total: (i64,) = match keyword {
            None => {
                sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(&self.pool)
                    .await
            }
            Some(k) => {
                let pattern = format!("%{}%", crate::utils::validation::escape_like_pattern(k));
                sqlx::query_as(
                    r#"SELECT COUNT(*) FROM users
                       WHERE username ILIKE $1 ESCAPE '\' OR email ILIKE $1 ESCAPE '\'"#,
                )
                .bind(&pattern)
                .fetch_one(&self.pool)
                .await
            }
        }
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
