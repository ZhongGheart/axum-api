//! 系统参数数据访问层 + Redis 缓存
//!
//! 与 [`crate::repository::dict`] 同构：DB 是权威源，Redis 只是缓存。
//! 读路径是 cache-aside（缓存未命中回查 DB 并回填），写路径**先写 DB 再删缓存**。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::setting::{find_def, SettingDef};
use crate::utils::redis::RedisClient;

/// 缓存键前缀
const SETTINGS_CACHE_PREFIX: &str = "settings:";
/// 缓存 TTL（秒）
///
/// 取 300 秒是个**兜底**而不是设计目标：正常情况下写路径会主动删缓存，
/// 多副本部署下靠 Redis 广播不到其他进程时才需要等它自然过期。
const SETTINGS_CACHE_TTL: u64 = 300;

/// 单个参数的持久化形态
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 读回来的一个参数：取值 + **它是谁说了算**
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettingEntry {
    /// 当前落库的取值
    pub value: String,
    /// 是否被管理员**显式改过**
    ///
    /// 由 `updated_by IS NOT NULL` 判定：种子写入的行 `updated_by` 为 NULL。
    /// 这个区分是 v0.22.0 最重要的一处设计（理由见 `service::setting`）。
    pub admin_overridden: bool,
}

/// 对外返回的参数视图（定义 + 当前取值）
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SettingView {
    /// 参数名
    pub key: String,
    /// 显示名
    pub name: String,
    /// 用途说明
    pub description: String,
    /// 取值类型（`int` / `bool`）
    pub value_type: String,
    /// 分组标识
    pub group: String,
    /// 当前取值（以文本表示）
    pub value: String,
    /// 默认值（以文本表示）
    pub default: String,
    /// 整数下界
    pub min: i64,
    /// 整数上界
    pub max: i64,
    /// 该参数被哪段代码消费
    pub consumed_by: String,
    /// 当前取值是否等于默认值
    pub is_default: bool,
    /// 是否被管理员显式改过（`updated_by IS NOT NULL`）
    pub admin_overridden: bool,
    /// 当前生效值的来源：`admin` / `env` / `default`
    ///
    /// 由 `SettingService::list` 填充（它才知道部署配置的值），
    /// 仓储层给 `default`。让管理员看清"这个数字到底是谁定的"。
    pub source: String,
    /// 最近修改者
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

impl SettingView {
    fn from_def(def: &SettingDef, row: &SettingRow) -> Self {
        Self {
            key: def.key.to_string(),
            name: def.name.to_string(),
            description: def.description.to_string(),
            value_type: def.value_type.as_str().to_string(),
            group: def.group.as_str().to_string(),
            value: row.value.clone(),
            default: def.default.to_string(),
            min: def.min,
            max: def.max,
            consumed_by: def.consumed_by.to_string(),
            is_default: row.value == def.default,
            admin_overridden: row.updated_by.is_some(),
            source: "default".to_string(),
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    }
}

/// 系统参数仓储
#[derive(Debug, Clone)]
pub struct SettingRepository {
    pool: PgPool,
    redis: Option<RedisClient>,
}

impl SettingRepository {
    pub fn new(pool: PgPool, redis: Option<RedisClient>) -> Self {
        Self { pool, redis }
    }

    /// 读取全部参数（key → 取值文本）
    ///
    /// 返回的是**以 key 为准**的合并结果：DB 里的每一行都要有一个
    /// [`SettingDef`]，而每个 [`SettingDef`] 都必须能在 DB 里找到取值。
    ///
    /// 两个方向都要补齐，理由不同：
    ///
    /// - DB 有、定义没有 → 该行是**孤儿**，忽略它。它可能是某个旧版本
    ///   留下的，或被绕过应用直接写进去的。让它进结果会让
    ///   "参数清单"随部署历史漂移。
    /// - 定义有、DB 没有 → 用 `default` 补上。**不能**当成错误：
    ///   迁移 `016` 用的是 `INSERT ... ON CONFLICT DO NOTHING`，
    ///   而运维完全可能在一个更早的备份上跑新二进制。
    ///   缺参数时退回默认值，与"参数不存在"的语义一致。
    pub async fn load_all(&self) -> Result<HashMap<String, SettingEntry>, AppError> {
        if let Some(redis) = &self.redis {
            let cache_key = format!("{SETTINGS_CACHE_PREFIX}all");
            match redis.get_string(&cache_key).await {
                Ok(Some(json)) => {
                    match serde_json::from_str::<HashMap<String, SettingEntry>>(&json) {
                        Ok(map) if !map.is_empty() => return Ok(map),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("参数缓存内容无法解析，回源查库: {e}"),
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("参数缓存读取失败，回源查库: {e}"),
            }
        }

        let rows: Vec<SettingRow> = sqlx::query_as(
            "SELECT key, value, updated_by, created_at, updated_at FROM system_settings",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询系统参数失败: {e}")))?;

        let mut map: HashMap<String, SettingEntry> = HashMap::new();
        for row in &rows {
            if find_def(&row.key).is_none() {
                tracing::warn!(
                    "system_settings 里的 {} 没有对应的参数定义，忽略（可能是旧版本残留）",
                    row.key
                );
                continue;
            }
            map.insert(
                row.key.clone(),
                SettingEntry {
                    value: row.value.clone(),
                    admin_overridden: row.updated_by.is_some(),
                },
            );
        }
        for def in crate::model::setting::SETTING_DEFS {
            map.entry(def.key.to_string())
                .or_insert_with(|| SettingEntry {
                    value: def.default.to_string(),
                    admin_overridden: false,
                });
        }

        if let Some(redis) = &self.redis {
            let cache_key = format!("{SETTINGS_CACHE_PREFIX}all");
            if let Ok(json) = serde_json::to_string(&map) {
                if let Err(e) = redis
                    .set_string(&cache_key, &json, SETTINGS_CACHE_TTL)
                    .await
                {
                    // 缓存写失败不影响读结论：数据已经查到了
                    tracing::warn!("参数缓存回填失败: {e}");
                }
            }
        }

        Ok(map)
    }

    /// 列出参数详情（按定义顺序）
    pub async fn list(&self) -> Result<Vec<SettingView>, AppError> {
        let map = self.load_all().await?;

        // `updated_by` / `updated_at` 需要单独查一次：它们不是被缓存的
        // "策略取值"，而是**元数据**。把元数据一起缓存会让"谁改的这个参数"
        // 在 TTL 内显示成旧的人，而参数管理页最需要看的就是这个。
        let rows: Vec<SettingRow> = sqlx::query_as(
            "SELECT key, value, updated_by, created_at, updated_at FROM system_settings",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询系统参数失败: {e}")))?;
        let meta: HashMap<String, SettingRow> =
            rows.into_iter().map(|r| (r.key.clone(), r)).collect();

        let mut views = Vec::with_capacity(crate::model::setting::SETTING_DEFS.len());
        for def in crate::model::setting::SETTING_DEFS {
            let value = map
                .get(def.key)
                .map(|e| e.value.clone())
                .unwrap_or_else(|| def.default.into());
            let row = meta.get(def.key).cloned().unwrap_or_else(|| SettingRow {
                key: def.key.to_string(),
                value: value.clone(),
                updated_by: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
            views.push(SettingView::from_def(def, &row));
        }
        Ok(views)
    }

    /// 更新单个参数
    ///
    /// 返回落库后的行；**写入前**由 service 层校验取值合法性，
    /// 这里只负责持久化。
    pub async fn update(&self, key: &str, value: &str, updated_by: Uuid) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO system_settings (key, value, updated_by)
            VALUES ($1, $2, $3)
            ON CONFLICT (key) DO UPDATE
            SET value = EXCLUDED.value,
                updated_by = EXCLUDED.updated_by,
                updated_at = NOW()
            "#,
        )
        .bind(key)
        .bind(value)
        .bind(updated_by)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("更新系统参数 {key} 失败: {e}")))?;

        // 写 DB 成功**之后**才删缓存。反过来的话：删成功、写失败，
        // 缓存被清空后下一次读会回源查到旧值，看起来"改了没生效"，
        // 而管理员已经收到 500。宁可让旧值多活 TTL，也不要制造假象。
        if let Some(redis) = &self.redis {
            let cache_key = format!("{SETTINGS_CACHE_PREFIX}all");
            if let Err(e) = redis.delete_key(&cache_key).await {
                tracing::warn!(
                    "参数缓存失效失败（新值可能在 {} 秒后才生效）: {e}",
                    SETTINGS_CACHE_TTL
                );
            }
        }
        Ok(())
    }

    /// 把某个参数复位成默认值
    pub async fn reset(&self, key: &str, updated_by: Uuid) -> Result<(), AppError> {
        let def = find_def(key)
            .ok_or_else(|| AppError::BadRequest(format!("没有名为 {key} 的系统参数")))?;
        self.update(key, def.default, updated_by).await
    }

    /// **交还控制权**：清掉管理员的覆盖，让部署配置重新说了算
    ///
    /// 与 [`Self::reset`] 的区别在语义：
    /// - `reset` = "把取值写成代码默认值"，仍然算**管理员的显式决定**
    /// - `clear_override` = "我不再管这个参数了"，交回给环境变量
    ///
    /// 对有环境变量的参数（登录锁定阈值/窗口），后者才是"复位"，
    /// 否则管理员点一次复位就把参数**钉死在**代码默认值上，
    /// 而部署侧改了环境变量从此不再有效果——这与本仓反复修的
    /// "配置写了却不生效"是同一类缺陷，只是方向相反。
    pub async fn clear_override(&self, key: &str) -> Result<(), AppError> {
        let def = find_def(key)
            .ok_or_else(|| AppError::BadRequest(format!("没有名为 {key} 的系统参数")))?;
        sqlx::query(
            r#"
            INSERT INTO system_settings (key, value, updated_by)
            VALUES ($1, $2, NULL)
            ON CONFLICT (key) DO UPDATE
            SET value = EXCLUDED.value,
                -- 置 NULL 就是"回到没被管理员改过的状态"，由 `updated_by IS NULL` 判定
                updated_by = NULL,
                updated_at = NOW()
            "#,
        )
        .bind(key)
        .bind(def.default)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("复位系统参数 {key} 失败: {e}")))?;

        if let Some(redis) = &self.redis {
            let cache_key = format!("{SETTINGS_CACHE_PREFIX}all");
            if let Err(e) = redis.delete_key(&cache_key).await {
                tracing::warn!("参数缓存失效失败: {e}");
            }
        }
        Ok(())
    }

    /// 清理参数缓存（「刷新参数」按钮的真实作用）
    pub async fn invalidate_cache(&self) -> Result<(), AppError> {
        let Some(redis) = &self.redis else {
            return Ok(());
        };
        let cache_key = format!("{SETTINGS_CACHE_PREFIX}all");
        redis.delete_key(&cache_key).await
    }
}
