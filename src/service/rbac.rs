//! 角色权限服务层
//!
//! 封装角色分配、初始化种子数据等业务逻辑。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::{permission, User};
use crate::repository::user::USER_COLUMNS;
use crate::utils::password::hash_password;

/// RBAC 种子数据初始化的事务级建议锁 key
const RBAC_SEED_LOCK_KEY: i64 = 0x5242_4143; // "RBAC"

/// 内置页面菜单树（固定 UUID 便于子节点引用父节点）
///
/// `component` 是前端 `src/views` 下的相对路径（不含 `.vue`），
/// 前端用 `import.meta.glob` 解析；此处新增条目必须对应真实存在的页面文件。
const SEED_MENUS_SQL: &str = r#"
INSERT INTO menus (id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible) VALUES
  ('7f000000-0000-4000-8000-000000000001', NULL,                                   '首页',     '/',                      'home/index',           'home',     1, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000002', NULL,                                   '组件示例', '/demo',                  NULL,                   'grid',     2, 'directory', NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000003', '7f000000-0000-4000-8000-000000000002', '前端组件', '/demo',                  'demo/index',           'grid',     1, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000004', '7f000000-0000-4000-8000-000000000002', '后端能力', '/demo/backend',          'demo/backend',         'grid',     2, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000005', '7f000000-0000-4000-8000-000000000002', '字典组件', '/demo/dict',             'demo/dict',            'grid',     3, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000006', NULL,                                   '系统管理', '/system',                NULL,                   'settings', 3, 'directory', NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000007', '7f000000-0000-4000-8000-000000000006', '用户管理', '/system/user',           'system/user/index',    'user',     1, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000008', '7f000000-0000-4000-8000-000000000006', '角色管理', '/system/role',           'system/role/index',    'role',     2, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000009', '7f000000-0000-4000-8000-000000000006', '菜单管理', '/system/menu',           'system/menu/index',    'settings', 3, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000010', '7f000000-0000-4000-8000-000000000006', '系统日志', '/system/log',            'system/log/index',     'settings', 4, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000011', '7f000000-0000-4000-8000-000000000006', '接口文档', '/system/api-docs',       'system/api-docs/index','settings', 5, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000012', '7f000000-0000-4000-8000-000000000006', '系统监控', '/system/monitor/system', 'monitor/system/index', 'settings', 6, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000013', '7f000000-0000-4000-8000-000000000006', '接口监控', '/system/monitor/api',    'monitor/api/index',    'settings', 7, 'menu',      NULL, TRUE),
  ('7f000000-0000-4000-8000-000000000014', '7f000000-0000-4000-8000-000000000006', '字典管理', '/system/dict',           'system/dict/index',    'settings', 8, 'menu',      NULL, TRUE)
ON CONFLICT (id) DO NOTHING
"#;

/// admin 拥有全部菜单
const SEED_ADMIN_MENUS_SQL: &str = r#"
INSERT INTO role_menus (role_id, menu_id)
SELECT r.id, m.id FROM roles r CROSS JOIN menus m WHERE r.name = 'admin'
ON CONFLICT (role_id, menu_id) DO NOTHING
"#;

/// user 只拥有通用页面（首页 + 组件示例及其子页）
const SEED_USER_MENUS_SQL: &str = r#"
INSERT INTO role_menus (role_id, menu_id)
SELECT r.id, m.id FROM roles r JOIN menus m
  ON m.path IN ('/', '/demo', '/demo/backend', '/demo/dict')
WHERE r.name = 'user'
ON CONFLICT (role_id, menu_id) DO NOTHING
"#;

/// 角色权限服务
#[derive(Debug, Clone)]
pub struct RbacService {
    /// 数据库连接池
    pool: sqlx::PgPool,
}

impl RbacService {
    /// 创建新的 RbacService 实例
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// 初始化默认角色和超级管理员
    ///
    /// 幂等操作：仅当 `roles` 表为空时执行。
    pub async fn init_defaults(&self) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务开启失败: {e}")))?;

        // 事务级建议锁：多副本同时首次启动时只允许一个进程执行种子写入，
        // 避免并发 INSERT 触发唯一约束冲突导致启动失败
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(RBAC_SEED_LOCK_KEY)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("获取种子锁失败: {e}")))?;

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM roles")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;

        if count.0 > 0 {
            tracing::info!("RBAC 已初始化，跳过角色种子数据");
            // 角色已在、但 menus 可能为空（v0.2 升级上来的库），仍需补菜单种子
            self.seed_navigation(&mut tx).await?;
            tx.commit()
                .await
                .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;
            return Ok(());
        }

        tracing::info!("开始初始化 RBAC 种子数据...");

        // 1. 创建默认角色
        sqlx::query(
            r#"
            INSERT INTO roles (name, description) VALUES
                ('admin', '系统管理员，拥有所有权限'),
                ('user',  '普通用户，基础访问权限')
            ON CONFLICT (name) DO NOTHING
            "#,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建默认角色失败: {e}")))?;

        // 2. 查找角色 ID
        let admin_role: (Uuid,) = sqlx::query_as("SELECT id FROM roles WHERE name = 'admin'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询 admin 角色失败: {e}")))?;

        let user_role: (Uuid,) = sqlx::query_as("SELECT id FROM roles WHERE name = 'user'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询 user 角色失败: {e}")))?;

        // 3. 创建默认超级管理员（如不存在）
        let admin_user = sqlx::query_as::<_, User>(&format!(
            r#"
            SELECT {USER_COLUMNS}
            FROM users
            WHERE username = 'admin'
            "#
        ))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询 admin 用户失败: {e}")))?;

        // 明文 → Argon2（v0.2 起取消客户端 SHA-256 预哈希）
        let password_hash =
            hash_password("admin123").map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let admin_id = if let Some(admin) = admin_user {
            // admin 已存在，更新密码哈希（兼容旧格式迁移）
            sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
                .bind(&password_hash)
                .bind(admin.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| AppError::InternalServerError(format!("更新 admin 密码失败: {e}")))?;
            admin.id
        } else {
            let new_admin: (Uuid,) = sqlx::query_as(
                r#"
                INSERT INTO users (username, email, password_hash, is_active)
                VALUES ('admin', 'admin@example.com', $1, true)
                RETURNING id
                "#,
            )
            .bind(&password_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("创建 admin 用户失败: {e}")))?;

            new_admin.0
        };

        // 4. 分配 admin + user 角色
        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
            ON CONFLICT (user_id, role_id) DO NOTHING
            "#,
        )
        .bind(admin_id)
        .bind(admin_role.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配 admin 角色失败: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
            ON CONFLICT (user_id, role_id) DO NOTHING
            "#,
        )
        .bind(admin_id)
        .bind(user_role.0)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配 user 角色失败: {e}")))?;

        self.seed_navigation(&mut tx).await?;

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("事务提交失败: {e}")))?;

        tracing::info!("RBAC 种子数据初始化完成");

        Ok(())
    }

    /// 导航相关种子（菜单树 + 权限码 + 角色授权）
    ///
    /// 无条件、幂等执行。拆成两步是因为二者的触发条件不同：
    /// 菜单树只在 `menus` 为空时写入（尊重管理员的增删），而权限码必须每次补齐
    /// （v0.3 升级上来的库 `menus` 非空，但没有 `type='button'` 行）。
    async fn seed_navigation(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), AppError> {
        self.seed_menus_if_empty(tx).await?;
        self.seed_permission_codes(tx).await
    }

    /// 菜单树种子数据（幂等：仅在 `menus` 为空时写入）
    ///
    /// 前端导航由后端菜单驱动，因此全新库与 v0.2 升级上来的空 `menus` 表
    /// 都要自带一份与内置页面一致的菜单树，否则登录后侧栏是空的。
    /// 角色的菜单授权同样在此建立：admin 全量、user 仅通用页面。
    async fn seed_menus_if_empty(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), AppError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM menus")
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询菜单失败: {e}")))?;

        if count.0 > 0 {
            return Ok(());
        }

        tracing::info!("开始初始化菜单种子数据...");

        for (label, sql) in [
            ("菜单", SEED_MENUS_SQL),
            ("admin 角色菜单", SEED_ADMIN_MENUS_SQL),
            ("user 角色菜单", SEED_USER_MENUS_SQL),
        ] {
            sqlx::query(sql)
                .execute(&mut **tx)
                .await
                .map_err(|e| AppError::InternalServerError(format!("初始化{label}失败: {e}")))?;
        }

        tracing::info!("菜单种子数据初始化完成");
        Ok(())
    }

    /// 权限码种子（幂等，可重复执行）
    ///
    /// 权限码是 `type='button'` 的菜单行，挂在对应页面菜单下：
    /// 1. 按 [`permission::PERMISSION_DEFS`] 逐条 upsert（以 `permission` 为幂等键）
    /// 2. 为 `admin` 授权**本次新建**的权限码
    ///
    /// 刻意用「按 `permission` 列 upsert」而不是固定 UUID：管理员可以在"菜单管理"页
    /// 自行创建权限码行，固定 UUID 会在重建时覆盖或冲突。
    ///
    /// **只授权新建行**是刻意的：若每次启动都把全部按钮权限塞回 admin，
    /// 管理员在「菜单管理」页撤销的授权会被下次启动悄悄恢复，撤权形同虚设。
    /// 新增权限码仍能开箱可用（本次新建 → 本次授权）。
    ///
    /// 幂等键需要一个唯一索引兜底，见迁移 `007`。
    async fn seed_permission_codes(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), AppError> {
        let mut created_ids: Vec<Uuid> = Vec::with_capacity(permission::PERMISSION_DEFS.len());

        for (index, def) in permission::PERMISSION_DEFS.iter().enumerate() {
            // 父菜单按 path 解析：页面菜单由 SEED_MENUS_SQL 写入，path 稳定
            let parent_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM menus WHERE path = $1 AND type <> 'button' LIMIT 1",
            )
            .bind(def.parent_path)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("解析权限码父菜单失败: {e}")))?;

            if parent_id.is_none() {
                tracing::warn!(
                    "权限码 {} 的父菜单路径 {} 不存在，跳过（页面菜单可能被管理员删除）",
                    def.code,
                    def.parent_path
                );
                continue;
            }

            // 以 permission 为幂等键：已存在则不覆盖管理员的改名/调整，
            // RETURNING 只返回真正新建的行（ON CONFLICT DO NOTHING 的既有行不回传）
            let rows: Vec<(Uuid,)> = sqlx::query_as(
                r#"
                INSERT INTO menus (id, parent_id, name, sort_order, type, permission, is_visible)
                VALUES (gen_random_uuid(), $1, $2, $3, 'button', $4, FALSE)
                ON CONFLICT (permission) WHERE permission IS NOT NULL AND permission <> '' DO NOTHING
                RETURNING id
                "#,
            )
            .bind(parent_id)
            .bind(def.name)
            .bind(index as i32)
            .bind(def.code)
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!("初始化权限码 {} 失败: {e}", def.code))
            })?;

            created_ids.extend(rows.into_iter().map(|(id,)| id));
        }

        if created_ids.is_empty() {
            tracing::debug!("权限码已全部存在，跳过授权（保留管理员已有的撤销）");
            return Ok(());
        }

        // admin 是数据上的超级用户（而非代码里的后门）：只对本次新建的权限码授权，
        // 因此管理员后续撤销的授权能持久生效
        sqlx::query(
            r#"
            INSERT INTO role_menus (role_id, menu_id)
            SELECT r.id, m.id
            FROM roles r
            JOIN menus m ON m.id = ANY($1)
            WHERE r.name = 'admin'
            ON CONFLICT (role_id, menu_id) DO NOTHING
            "#,
        )
        .bind(&created_ids)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::InternalServerError(format!("分配 admin 权限码失败: {e}")))?;

        tracing::info!(
            "权限码种子数据初始化完成（新建 {} 条并授权 admin）",
            created_ids.len()
        );
        Ok(())
    }
}
