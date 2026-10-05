//! 菜单数据访问层

use std::collections::{HashMap, HashSet};

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{Menu, MenuNode, UnreachableMenu};

/// 菜单仓储
#[derive(Debug, Clone)]
pub struct MenuRepository {
    pool: PgPool,
}

/// `Menu` 的列清单
///
/// `sqlx::FromRow` 要求结果集**包含结构体的每一个字段**，少一列就在运行时报错。
/// 本仓库有 4 处把 `menus` 行映射成 `Menu`，手写 4 份列名迟早会漏——
/// 迁移 009 加列时就差点只改一半。集中成常量，加列只改这一处。
const MENU_COLUMNS: &str = "id, parent_id, name, path, component, icon, sort_order, type, \
     permission, is_visible, created_at, updated_at, prev_permission, \
     prev_permission_cleared_by";

/// [`MENU_COLUMNS`] 的带表别名版本（供 `menus m` 这类 JOIN 查询使用）
fn prefixed_menu_columns() -> String {
    MENU_COLUMNS
        .split(", ")
        .map(|c| format!("m.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 把写路径上的数据库约束冲突翻译成可操作的业务错误
///
/// 有两条约束原本会以 500「服务器内部错误」的形式冒到界面上，
/// 而它们都是**入参问题**，不是服务端故障：
///
/// - `idx_menus_permission_unique`（迁移 `007`）：权限码必须唯一
/// - `menus_type_check`：`type` 只能是 `menu` / `button` / `directory`
///
/// 入参错误报 500 有两个代价：错误监控被入参噪声污染，
/// 且管理员在界面上只看到「服务器内部错误」，完全不知道该怎么改。
///
/// 唯一索引还有一个额外作用：并发下两个请求可能都通过了
/// controller 的占用预查，最终由索引裁决——那条路也必须报冲突而不是 500。
fn map_write_violation(e: sqlx::Error, fallback: String) -> AppError {
    match e.as_database_error().and_then(|db| db.constraint()) {
        Some("idx_menus_permission_unique") => {
            AppError::Conflict("该权限码已被其他菜单使用".to_string())
        }
        Some("menus_type_check") => {
            AppError::BadRequest("菜单类型只能是 menu / button / directory 之一".to_string())
        }
        _ => AppError::InternalServerError(fallback),
    }
}

impl MenuRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 查询所有菜单（平坦列表）
    pub async fn find_all(&self) -> Result<Vec<Menu>, AppError> {
        sqlx::query_as::<_, Menu>(&format!(
            "SELECT {MENU_COLUMNS} FROM menus ORDER BY sort_order ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询菜单失败: {e}")))
    }

    /// 构建菜单树（全量）
    pub async fn find_tree(&self) -> Result<Vec<MenuNode>, AppError> {
        let all = self.find_all().await?;
        Ok(build_tree(&all))
    }

    /// 根据角色 ID 查询菜单 ID 列表
    pub async fn find_menu_ids_by_role(&self, role_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let ids =
            sqlx::query_scalar::<_, Uuid>("SELECT menu_id FROM role_menus WHERE role_id = $1")
                .bind(role_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::InternalServerError(format!("查询角色菜单失败: {e}")))?;
        Ok(ids)
    }

    /// 根据角色 ID 查询菜单树（仅返回有权限的节点）
    ///
    /// 注意：授权集合通常是**不完整**的——用户可能只勾了某个菜单页而没勾它的
    /// 上级目录。`build_tree` 会把「父节点不在集合内」的节点当作根返回，
    /// 而不是悄悄丢弃（见 `build_tree` 的说明）。
    pub async fn find_tree_by_role(&self, role_id: Uuid) -> Result<Vec<MenuNode>, AppError> {
        let menu_ids = self.find_menu_ids_by_role(role_id).await?;
        let all = self.find_all().await?;
        let filtered: Vec<Menu> = all
            .into_iter()
            .filter(|m| menu_ids.contains(&m.id))
            .collect();
        Ok(build_tree(&filtered))
    }

    /// 查询多个角色可见的**导航菜单树**
    ///
    /// - 合并用户所有角色的菜单并去重
    /// - 只返回 `is_visible = true` 且非按钮（`type <> 'button'`）的节点，
    ///   按钮型菜单是权限标记，不应出现在导航里
    /// - 排序沿用 `sort_order`
    pub async fn find_tree_for_roles(&self, role_ids: &[Uuid]) -> Result<Vec<MenuNode>, AppError> {
        if role_ids.is_empty() {
            return Ok(Vec::new());
        }

        let menus = sqlx::query_as::<_, Menu>(&format!(
            r#"
                SELECT DISTINCT {}
                FROM menus m
                JOIN role_menus rm ON rm.menu_id = m.id
                WHERE rm.role_id = ANY($1)
                  AND m.is_visible = TRUE
                  AND m.type <> 'button'
                ORDER BY m.sort_order ASC
                "#,
            prefixed_menu_columns()
        ))
        .bind(role_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户菜单失败: {e}")))?;

        Ok(build_tree(&menus))
    }

    /// 校验「把 `moving` 挂到 `parent` 下」在**结构上**是否合法
    ///
    /// `menus.parent_id` 的外键只能挡住"指向不存在的菜单"，
    /// **挡不住成环**——而环的后果实测是三级放大：
    ///
    /// 1. `build_tree` 判根看"父节点是否在集合内"，环上无一为根 → **整支被剪掉**，
    ///    菜单从侧栏和管理页同时消失，管理员在界面上看不见它，也就无法点开改回来
    /// 2. `granted_codes_in_subtree` 的递归 CTE 遇到环**永不收敛** →
    ///    `DELETE /api/admin/menus/:id` 永久挂起，连"删掉它"这条自救路也堵死
    /// 3. 挂起的请求不归还连接，占满连接池后**与菜单无关的端点也全部 500**
    ///
    /// `moving = None` 表示新建菜单（新节点不可能是任何节点的祖先，只需父级存在）。
    pub async fn ensure_attachable(
        &self,
        moving: Option<Uuid>,
        parent: Uuid,
    ) -> Result<(), AppError> {
        let all = self.find_all().await?;

        // 先给父级一个能照着改的消息，而不是让 FK 抛 500「服务器内部错误」
        let Some(parent_menu) = all.iter().find(|m| m.id == parent) else {
            return Err(AppError::BadRequest(
                "上级菜单不存在，请刷新菜单列表后重新选择".into(),
            ));
        };

        let Some(moving_id) = moving else {
            return Ok(());
        };

        if moving_id == parent {
            return Err(AppError::BadRequest("上级菜单不能是它自己".into()));
        }

        // 沿 parent 向上走祖先链：只要经过 moving_id，就会成环。
        //
        // **刻意在 Rust 侧走而不用 SQL 递归**：库里可能**已经**存在环
        // （历史脏数据，或运维直连 DB 写入）。任何不带 visited 的向上遍历
        // 在那种数据上自己就会死循环——用 `WITH RECURSIVE` 同样躲不掉，
        // 那正是 [`Self::granted_codes_in_subtree`] 曾经挂起的原因。
        let parent_of: HashMap<Uuid, Option<Uuid>> =
            all.iter().map(|m| (m.id, m.parent_id)).collect();
        let mut seen = HashSet::from([moving_id]);
        let mut cursor = parent_menu.parent_id;

        while let Some(id) = cursor {
            if id == moving_id {
                return Err(AppError::BadRequest(
                    "不能把菜单挪到它自己的下级里：那会让菜单树成环，整棵子树会从界面上消失".into(),
                ));
            }
            // 祖先链上已有环（与本次操作无关的脏数据）：到此为止，不要跟着转圈
            if !seen.insert(id) {
                break;
            }
            cursor = parent_of.get(&id).copied().flatten();
        }

        Ok(())
    }

    /// 查询**从根节点出发走不到**的菜单（结构已损坏、但仍留在库里）
    ///
    /// `build_tree` 会把这类节点静默剪掉，所以管理员在菜单页看不到它们——
    /// 这正是 v0.16.0 之前"成环后无法自救"的由来：本接口让它们**被看见**。
    ///
    /// 损坏来源有两个，修复动作相同（挂到根下），所以合并成一类：
    /// - **成环**：FK 挡不住，`update_menu` 曾经也不校验
    /// - **孤儿**：`parent_id` 指向不存在的菜单（FK 已堵住，此处兜底）
    pub async fn find_unreachable(&self) -> Result<Vec<UnreachableMenu>, AppError> {
        let all = self.find_all().await?;
        let exists: HashSet<Uuid> = all.iter().map(|m| m.id).collect();

        // 从根（parent_id IS NULL）出发做前沿扩展，标记可达集合。
        // 用 HashSet 去重：即便库里已经有环，也不会转圈停不下来。
        let mut reachable: HashSet<Uuid> = HashSet::new();
        let mut frontier: Vec<Uuid> = all
            .iter()
            .filter(|m| m.parent_id.is_none())
            .map(|m| m.id)
            .collect();
        reachable.extend(frontier.iter().copied());

        while let Some(id) = frontier.pop() {
            for m in all.iter().filter(|m| m.parent_id == Some(id)) {
                if reachable.insert(m.id) {
                    frontier.push(m.id);
                }
            }
        }

        Ok(all
            .iter()
            .filter(|m| !reachable.contains(&m.id))
            .map(|m| UnreachableMenu {
                id: m.id,
                name: m.name.clone(),
                parent_id: m.parent_id,
                reason: match m.parent_id {
                    // 有父级且父级存在却仍不可达 ⇒ 只可能是成环
                    Some(pid) if exists.contains(&pid) => "菜单树成环，无法从根节点到达",
                    _ => "上级菜单不存在（悬空引用）",
                }
                .to_string(),
                sort_order: m.sort_order,
            })
            .collect())
    }

    /// 新增菜单
    pub async fn create(&self, menu: &Menu) -> Result<Menu, AppError> {
        // 新建节点不可能是任何节点的祖先（id 全新），但父级存在性仍要自己查：
        // FK 抛出来的是 500「服务器内部错误」，入参问题不该伪装成服务端故障。
        if let Some(parent) = menu.parent_id {
            self.ensure_attachable(None, parent).await?;
        }

        sqlx::query_as::<_, Menu>(
            &format!(
                r#"
                INSERT INTO menus (id, parent_id, name, path, component, icon, sort_order, type, permission, is_visible)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING {MENU_COLUMNS}
                "#
            ),
        )
        .bind(menu.id)
        .bind(menu.parent_id)
        .bind(&menu.name)
        .bind(&menu.path)
        .bind(&menu.component)
        .bind(&menu.icon)
        .bind(menu.sort_order)
        .bind(&menu.r#type)
        .bind(&menu.permission)
        .bind(menu.is_visible)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            let fallback = format!("创建菜单失败: {e}");
            map_write_violation(e, fallback)
        })
    }

    /// 查询给定角色集合拥有的权限码
    ///
    /// 权限码是 `type = 'button'` 菜单行的 `permission` 列，经 `role_menus` 授权。
    /// 单条索引 JOIN 完成，无 N+1：
    ///
    /// ```sql
    /// SELECT DISTINCT m.permission
    /// FROM menus m
    /// JOIN role_menus rm ON rm.menu_id = m.id
    /// JOIN roles r      ON r.id = rm.role_id
    /// WHERE r.name = ANY($1) AND m.type = 'button'
    ///   AND m.permission IS NOT NULL AND m.permission <> ''
    /// ```
    ///
    /// 刻意不做缓存：撤销角色菜单授权后必须立即生效，
    /// 避免 TTL 窗口内出现已撤权仍可调用的情况。
    pub async fn find_permission_codes(
        &self,
        role_names: &[String],
    ) -> Result<Vec<String>, AppError> {
        if role_names.is_empty() {
            return Ok(Vec::new());
        }

        let codes = sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT m.permission
            FROM menus m
            JOIN role_menus rm ON rm.menu_id = m.id
            JOIN roles r      ON r.id = rm.role_id
            WHERE r.name = ANY($1)
              AND m.type = 'button'
              AND m.permission IS NOT NULL
              AND m.permission <> ''
            ORDER BY m.permission ASC
            "#,
        )
        .bind(role_names)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询用户权限码失败: {e}")))?;

        Ok(codes)
    }

    /// 查询指定菜单集合携带的权限码（v0.5.0 PR-3：授权下界判定用）
    ///
    /// `assign_role_menus` 要判断"调用者能否把这些菜单授予该角色"，
    /// 必须先把菜单集合翻译成权限码集合再和调用者已持有的码比较。
    ///
    /// 与 [`Self::find_permission_codes`] 用同一条过滤条件
    /// （`type = 'button'` 且 `permission` 非空）：**只有按钮行携带权限码**，
    /// 目录/页面菜单只影响导航可见性，不授予任何接口调用能力，因此不参与判定。
    /// 否则"整理菜单结构"这种无害操作也会被误判为提权。
    pub async fn find_permission_codes_by_menu_ids(
        &self,
        menu_ids: &[Uuid],
    ) -> Result<Vec<String>, AppError> {
        if menu_ids.is_empty() {
            return Ok(Vec::new());
        }

        let codes = sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT permission
            FROM menus
            WHERE id = ANY($1)
              AND type = 'button'
              AND permission IS NOT NULL
              AND permission <> ''
            ORDER BY permission ASC
            "#,
        )
        .bind(menu_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询菜单权限码失败: {e}")))?;

        Ok(codes)
    }

    /// 查询**单个角色**当前持有的全部权限码
    ///
    /// v0.13.0 审计用：`assign_role_menus` 是全量替换语义，
    /// 要记下"这次授了/撤了哪些码"，只能在替换前后各取一次快照求差。
    /// 只记提交上来的菜单 ID 答不出"撤了哪些"——
    /// 而撤销恰恰是事后追溯最想知道的那一半。
    ///
    /// 过滤条件与 [`Self::find_permission_codes`] 一致
    /// （`type = 'button'` 且 `permission` 非空）：目录/页面菜单不带码，
    /// 计入变更只会制造没有权限含义的噪声。
    pub async fn permission_codes_of_role(&self, role_id: Uuid) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT m.permission
            FROM menus m
            JOIN role_menus rm ON rm.menu_id = m.id
            WHERE rm.role_id = $1
              AND m.type = 'button'
              AND m.permission IS NOT NULL
              AND m.permission <> ''
            ORDER BY m.permission ASC
            "#,
        )
        .bind(role_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询角色权限码失败: {e}")))
    }

    /// 把一组权限码映射回持有它们的菜单 ID（v0.26.0 审计用）
    ///
    /// `assign_role_menus` 求差得到的是**权限码**（`system:user:list`），
    /// 而结构化审计要记的是**菜单**（`target_type = 'menu'` + 菜单 UUID）。
    /// 两者之间缺这一步就接不上。
    ///
    /// **一个码可能对应多个菜单**，因此返回的是 `Vec<(Uuid, String)>`：
    /// 去重成"一个码一个菜单"会漏掉另一个同样声明了该码的按钮，
    /// 而"谁动过这个按钮"恰恰是要回答的问题。
    /// 找不到的码（菜单已被删除）**静默不出现在结果里**——
    /// 此时那条 target 无法归属到任何现存菜单，但摘要文本里仍留着码名。
    pub async fn find_menu_ids_by_permission_codes(
        &self,
        codes: &[String],
    ) -> Result<Vec<(Uuid, String)>, AppError> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, (Uuid, String)>(
            r#"
            SELECT id, permission
            FROM menus
            WHERE permission = ANY($1)
              AND type = 'button'
            ORDER BY permission ASC, id ASC
            "#,
        )
        .bind(codes)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("按权限码反查菜单失败: {e}")))?;
        Ok(rows)
    }

    /// 更新菜单
    ///
    /// `actor_id` 只用于权限码留痕：把某个码从按钮上清空时，
    /// 记下"清掉的是什么、谁清的"，供 `restore_permission` 让本人撤销误操作。
    pub async fn update(
        &self,
        id: Uuid,
        fields: &crate::model::UpdateMenuRequest,
        actor_id: Uuid,
    ) -> Result<Menu, AppError> {
        let menu = self.find_by_id(id).await?;
        // `parent_id` 是三态的（见 `UpdateMenuRequest`）：
        // 没传 → 保持原样；`Some(None)` → 摘成根；`Some(Some(pid))` → 挂到 pid 下。
        //
        // 只在父级**真的变更**时校验"挂得上去吗"——FK 挡不住成环，
        // 而成环会让整棵子树静默消失、删除永久挂起（见 `ensure_attachable`）。
        //
        // 刻意**不**校验"父级没变"的情况：那种数据本来就已经在库里了，
        // 拒绝这次写入既救不了它，还会顺带把改名、改图标这类无害操作也堵死——
        // 包括"把环上的节点摘成根"这条唯一的自救操作（它走 `Some(None)`，
        // 本就不该被拦）。
        let parent_id = match &fields.parent_id {
            Some(new_parent) => *new_parent,
            None => menu.parent_id,
        };
        if let Some(parent) = parent_id {
            self.ensure_attachable(Some(id), parent).await?;
        }
        let name = fields.name.as_deref().unwrap_or(&menu.name);
        let path = fields.path.as_deref().or(menu.path.as_deref());
        let component = fields.component.as_deref().or(menu.component.as_deref());
        let icon = fields.icon.as_deref().or(menu.icon.as_deref());
        let sort_order = fields.sort_order.unwrap_or(menu.sort_order);
        let r#type = fields.r#type.as_deref().unwrap_or(&menu.r#type);
        // 空串与 NULL 在授权查询里等价（都靠 `permission <> ''` 过滤掉），
        // 但统一落成 NULL，"这个按钮当前有没有码"才是一个确定的事实，
        // 不会在恢复逻辑里出现"空串算不算已清空"的分支。
        let permission = fields
            .permission
            .as_deref()
            .filter(|p| !p.is_empty())
            .or_else(|| {
                // `fields.permission` 显式给了空串 ⇒ 本次就是清空，不能回退旧值
                if fields.permission.is_some() {
                    None
                } else {
                    menu.permission.as_deref().filter(|p| !p.is_empty())
                }
            });
        let is_visible = fields.is_visible.unwrap_or(menu.is_visible);

        // 恢复槽位的记账规则（只有权限码真的变了才动）：
        //
        // - 本次把某个码清掉了 → 记进 `prev_permission`，供恢复
        // - 本次把码设成了新值 → 槽位作废（按钮现在有码了，没有"丢失"可恢复）
        // - 本次没碰 `permission` → 原样保留，别让一次改名把恢复凭据冲掉
        let prev_permission = if permission.is_some() {
            None
        } else if menu.permission.is_some() {
            menu.permission.clone()
        } else {
            menu.prev_permission.clone()
        };
        let prev_permission_cleared_by = if permission.is_some() {
            None
        } else if menu.permission.is_some() {
            Some(actor_id)
        } else {
            menu.prev_permission_cleared_by
        };

        sqlx::query_as::<_, Menu>(&format!(
            r#"
                UPDATE menus SET parent_id=$2, name=$3, path=$4, component=$5, icon=$6,
                    sort_order=$7, type=$8, permission=$9, is_visible=$10,
                    prev_permission=$11, prev_permission_cleared_by=$12
                WHERE id=$1
                RETURNING {MENU_COLUMNS}
                "#
        ))
        .bind(id)
        .bind(parent_id)
        .bind(name)
        .bind(path)
        .bind(component)
        .bind(icon)
        .bind(sort_order)
        .bind(r#type)
        .bind(permission)
        .bind(is_visible)
        .bind(prev_permission)
        .bind(prev_permission_cleared_by)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            let fallback = format!("更新菜单失败: {e}");
            map_write_violation(e, fallback)
        })
    }

    /// 把 `prev_permission` 写回 `permission`
    ///
    /// 调用方（controller）负责校验"是本人清空的"，仓储只管执行。
    /// 写回后清空恢复槽位：凭据只能用一次，否则"清空→恢复→再清空"
    /// 会让槽位指向一个已经不在按钮上的码。
    pub async fn restore_permission(&self, id: Uuid) -> Result<Menu, AppError> {
        sqlx::query_as::<_, Menu>(&format!(
            r#"
            UPDATE menus
            SET permission = prev_permission,
                prev_permission = NULL,
                prev_permission_cleared_by = NULL
            WHERE id = $1 AND prev_permission IS NOT NULL
            RETURNING {MENU_COLUMNS}
            "#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("恢复菜单权限码失败: {e}")))?
        .ok_or_else(|| AppError::BadRequest("该菜单没有可恢复的权限码".into()))
    }

    /// 该权限码是否已被某个菜单占用
    ///
    /// 用于把"声明一个已被占用的权限码"翻译成可操作的冲突消息，
    /// 而不是让唯一索引抛出 500。见 [`Self::create`]。
    pub async fn is_permission_taken(&self, code: &str) -> Result<bool, AppError> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM menus WHERE permission = $1)")
            .bind(code)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询权限码占用状态失败: {e}")))
    }

    /// 该菜单**及其整棵子树**里，所有"已被授予至少一个角色"的权限码
    ///
    /// 删除是级联的（见 [`Self::delete`]）：`menus.parent_id` 声明了
    /// `ON DELETE CASCADE`，删一个目录会连带删掉整棵子树。
    /// 因此删除守卫必须看整棵子树——只看目标节点自身的 `permission`
    /// 会漏掉"删父目录、连带删掉子树里承载码的按钮"这条路，
    /// 而它与直接删那个按钮的效果完全相同。
    ///
    /// 只返回**已授予**的码：没人依赖的码删掉不改变任何人的权限，
    /// 不该计入守卫要求（否则"整理菜单结构"这类无害操作会全线报错）。
    ///
    /// 递归项用 **`UNION` 而非 `UNION ALL`**：`UNION ALL` 不去重，
    /// 遇到成环数据会**永不收敛**，这个查询不返回 → `DELETE /api/admin/menus/:id`
    /// 永久挂起且连接不归还 → 占满连接池后全站 500（v0.17.0 实测复现）。
    /// `UNION` 按 `id` 去重，环上转一圈就停，语义在无环数据上与 `UNION ALL` 完全一致。
    /// [`Self::ensure_attachable`] 负责让环**进不来**，这里是第二道防线。
    pub async fn granted_codes_in_subtree(&self, id: Uuid) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar(
            r#"
            WITH RECURSIVE subtree AS (
                SELECT id FROM menus WHERE id = $1
                UNION
                SELECT m.id FROM menus m JOIN subtree s ON m.parent_id = s.id
            )
            SELECT DISTINCT m.permission
            FROM menus m
            JOIN subtree s ON m.id = s.id
            JOIN role_menus rm ON rm.menu_id = m.id
            WHERE m.permission IS NOT NULL AND m.permission <> ''
            ORDER BY m.permission
            "#,
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询菜单子树被授予的权限码失败: {e}")))
    }

    /// 该菜单是否已被授予至少一个角色
    ///
    /// 用于判定"清空权限码"到底有没有改变任何人的权限：
    /// 已授予 → 清空就是从那些角色手里收回码，属于授权操作，要过守卫；
    /// 未授予 → 不改变任何人的权限，放行。
    pub async fn is_granted_to_any_role(&self, id: Uuid) -> Result<bool, AppError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM role_menus WHERE menu_id = $1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::InternalServerError(format!("查询菜单授权状态失败: {e}")))?;
        Ok(exists)
    }

    /// 删除菜单
    ///
    /// 子节点与 `role_menus` 的清理**交给数据库外键级联**：
    /// `menus.parent_id` 与 `role_menus.menu_id` 都声明了 `ON DELETE CASCADE`，
    /// 单条 DELETE 即原子地删掉整棵子树及其全部角色授权。
    ///
    /// 早期实现在这里手写递归删除子节点，且每条语句都用 `.ok()` 吞掉错误：
    /// 那些语句既多余（外键已经级联），又把真实错误藏了起来。
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        // 不存在的菜单返回 404，而不是"删除成功"——UI 上的删除按钮需要能区分两者
        let rows = sqlx::query("DELETE FROM menus WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除菜单失败: {e}")))?;

        if rows.rows_affected() == 0 {
            return Err(AppError::NotFound("菜单不存在".into()));
        }
        Ok(())
    }

    /// 根据 ID 查询
    pub async fn find_by_id(&self, id: Uuid) -> Result<Menu, AppError> {
        sqlx::query_as::<_, Menu>(&format!("SELECT {MENU_COLUMNS} FROM menus WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询菜单失败: {e}")))?
            .ok_or_else(|| AppError::NotFound("菜单不存在".into()))
    }

    /// 分配角色菜单权限（全量替换）
    ///
    /// 全量替换语义：先清空该角色的全部授权，再写入新集合。
    /// 三步在同一事务内，**任何一步失败都整体回滚**。
    ///
    /// 早期实现对 DELETE / INSERT 都用 `.ok()` 吞掉错误却仍然 `commit()`，后果是：
    /// - 撤销静默失效——取消勾选、保存成功，权限其实还在
    /// - 传入"合法 + 非法"混合 ID 时静默**部分**授权，却返回"权限分配成功"
    ///
    /// 授权写路径必须"要么完整成功、要么整体失败"：半吊子的授权比没有授权更危险，
    /// 因为管理员会以为撤销已经生效。
    pub async fn assign_role_menus(
        &self,
        role_id: Uuid,
        menu_ids: &[Uuid],
    ) -> Result<(), AppError> {
        // 角色不存在要给出明确的 404，而不是等外键约束报错
        let role_exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM roles WHERE id = $1")
            .bind(role_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("查询角色失败: {e}")))?;
        if role_exists.is_none() {
            return Err(AppError::NotFound("角色不存在".into()));
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query("DELETE FROM role_menus WHERE role_id = $1")
            .bind(role_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清空角色原有授权失败: {e}")))?;

        if !menu_ids.is_empty() {
            // 单条 `INSERT ... SELECT unnest` 取代逐条循环：一次往返而非 N 次，
            // 且任一 ID 非法（外键不存在）会让整条语句失败 → 事务回滚，
            // 不会留下"只授权了一半"的中间态。
            sqlx::query(
                "INSERT INTO role_menus (role_id, menu_id)
                 SELECT $1, unnest($2::uuid[])
                 ON CONFLICT DO NOTHING",
            )
            .bind(role_id)
            .bind(menu_ids)
            .execute(&mut *tx)
            .await
            .map_err(map_menu_fk_error)?;
        }

        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))
    }
}

/// 递归构建菜单树（森林）
///
/// **入参常常是不完整的子集**（按角色过滤后的授权集合、可见且非按钮的导航集合），
/// 因此判定「根节点」不能只看 `parent_id IS NULL`，而要看**父节点是否在集合内**：
/// 父节点不在集合里的节点一律当作根返回。
///
/// 早先这里只过滤 `parent_id == None`，后果是「勾了子菜单但没勾上级目录」
/// 的角色在 `GET /api/admin/menus?role_id=` 下返回**空树**——
/// 前端授权弹窗会显示「该角色没有任何权限」，管理员一保存就把授权全清空。
/// admin 因为被种子授满全部菜单（含所有祖先）而恰好看不出问题。
fn build_tree(all: &[Menu]) -> Vec<MenuNode> {
    let ids: std::collections::HashSet<Uuid> = all.iter().map(|m| m.id).collect();
    let is_root = |m: &Menu| match m.parent_id {
        Some(pid) => !ids.contains(&pid),
        None => true,
    };

    all.iter()
        .filter(|m| is_root(m))
        .map(|m| {
            let mut node = MenuNode::from(m.clone());
            node.children = build_children(all, m.id);
            node
        })
        .collect()
}

/// 递归收集 `parent` 的直接子节点
fn build_children(all: &[Menu], parent: Uuid) -> Vec<MenuNode> {
    all.iter()
        .filter(|m| m.parent_id == Some(parent))
        .map(|m| {
            let mut node = MenuNode::from(m.clone());
            node.children = build_children(all, m.id);
            node
        })
        .collect()
}

/// 把菜单授权写入时的数据库错误翻译成可操作的提示
///
/// `role_menus.menu_id` 有指向 `menus(id)` 的外键，因此传入不存在的菜单 ID 会触发
/// `23503`（foreign_key_violation）。这类错误是**调用方的问题**（传错了 ID），
/// 应返回 400 而不是让人从"服务器内部错误: ..."里猜。
fn map_menu_fk_error(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.code().as_deref() == Some("23503") {
            return AppError::BadRequest("提交的菜单 ID 不存在，请刷新后重试".into());
        }
    }
    AppError::InternalServerError(format!("写入角色授权失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::build_tree;
    use crate::model::Menu;
    use uuid::Uuid;

    fn menu(id: Uuid, parent: Option<Uuid>) -> Menu {
        let now = chrono::Utc::now();
        Menu {
            id,
            parent_id: parent,
            name: id.to_string(),
            path: None,
            component: None,
            icon: None,
            sort_order: 0,
            r#type: "menu".into(),
            permission: None,
            is_visible: true,
            created_at: now,
            updated_at: now,
            prev_permission: None,
            prev_permission_cleared_by: None,
        }
    }

    fn flatten(nodes: &[crate::model::MenuNode], out: &mut Vec<Uuid>) {
        for n in nodes {
            out.push(n.id);
            flatten(&n.children, out);
        }
    }

    #[test]
    fn build_tree_roots_nodes_whose_parent_is_absent() {
        // 授权集合里只有「角色管理」和它的按钮，没有上级目录「系统管理」
        let dir = Uuid::new_v4();
        let page = Uuid::new_v4();
        let btn = Uuid::new_v4();
        let subset = vec![menu(page, Some(dir)), menu(btn, Some(page))];

        let tree = build_tree(&subset);

        // 页面必须以根出现，而不是被悄悄丢掉
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, page);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].id, btn);
    }

    #[test]
    fn build_tree_returns_every_node_exactly_once() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let full = vec![menu(a, None), menu(b, Some(a)), menu(c, Some(b))];

        let tree = build_tree(&full);
        let mut ids = Vec::new();
        flatten(&tree, &mut ids);

        assert_eq!(ids.len(), 3, "每个节点应恰好出现一次: {ids:?}");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn build_tree_of_empty_set_is_empty() {
        assert!(build_tree(&[]).is_empty());
    }

    #[test]
    fn build_tree_of_orphan_children_only_is_not_empty() {
        // 只授权了按钮、连菜单页都没授权：早先的实现这里会返回空树
        let page = Uuid::new_v4();
        let btn = Uuid::new_v4();
        let tree = build_tree(&[menu(btn, Some(page))]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, btn);
    }
}
