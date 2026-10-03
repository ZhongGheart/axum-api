//! 数据字典数据访问层 + Redis 缓存

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{DictItem, DictItemResponse, DictType, DictTypeWithItems};
use crate::utils::redis::RedisClient;

const DICT_CACHE_PREFIX: &str = "dict:";

/// 把字典项写入时的约束冲突翻译成人话
///
/// 为什么要专门做这一层：仓储层已经在写路径上"先取消旧默认项"，
/// 但那是应用层保证——两个并发请求会各自看到"现在还没有默认项"，
/// 最后各写一个，由迁移 012 的部分唯一索引裁决。
/// 那条路径如果不翻译，管理员看到的是"服务器内部错误"，
/// 既不知道发生了什么，也不知道该改什么。
/// 沿用 `repository/menu.rs` 的 `map_write_violation` 同一套路。
fn map_dict_item_write_violation(e: sqlx::Error, fallback: String) -> AppError {
    match e.as_database_error().and_then(|db| db.constraint()) {
        Some("idx_dict_items_single_default") => {
            AppError::Conflict("该字典已有一个默认项，请稍后重试或先取消原有默认项".to_string())
        }
        _ => AppError::InternalServerError(fallback),
    }
}

/// 字典仓储
#[derive(Debug, Clone)]
pub struct DictRepository {
    pool: PgPool,
    redis: Option<RedisClient>,
}

impl DictRepository {
    pub fn new(pool: PgPool, redis: Option<RedisClient>) -> Self {
        Self { pool, redis }
    }

    // ── 字典类型 ────────────────────────────────────────

    pub async fn list_types(&self) -> Result<Vec<DictType>, AppError> {
        sqlx::query_as::<_, DictType>(
            "SELECT id, code, name, description, status, sort_order, created_at, updated_at FROM dict_types ORDER BY sort_order ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询字典类型失败: {e}")))
    }

    pub async fn find_type_by_code(&self, code: &str) -> Result<Option<DictType>, AppError> {
        sqlx::query_as::<_, DictType>(
            "SELECT id, code, name, description, status, sort_order, created_at, updated_at FROM dict_types WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询字典类型失败: {e}")))
    }

    pub async fn create_type(&self, t: &DictType) -> Result<DictType, AppError> {
        sqlx::query_as::<_, DictType>(
            "INSERT INTO dict_types (id, code, name, description, status, sort_order) VALUES ($1,$2,$3,$4,$5,$6) RETURNING *",
        )
        .bind(t.id).bind(&t.code).bind(&t.name).bind(&t.description).bind(&t.status).bind(t.sort_order)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建字典类型失败: {e}")))
    }

    pub async fn update_type(
        &self,
        id: Uuid,
        fields: &crate::model::CreateDictTypeRequest,
    ) -> Result<DictType, AppError> {
        let old = self.find_type_by_id(id).await?;
        let old_code = old.code.clone();
        let code = &fields.code;
        let name = &fields.name;
        let desc = fields.description.as_deref().or(old.description.as_deref());
        let status = fields.status.as_deref().unwrap_or(&old.status);
        let sort = fields.sort_order.unwrap_or(old.sort_order);
        let saved = sqlx::query_as::<_, DictType>(
            "UPDATE dict_types SET code=$2,name=$3,description=$4,status=$5,sort_order=$6 WHERE id=$1 RETURNING *",
        )
        .bind(id).bind(code).bind(name).bind(desc).bind(status).bind(sort)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("更新字典类型失败: {e}")))?;

        // 编码可能变化：新旧两个缓存键都要失效
        self.invalidate_cache(&old_code).await;
        self.invalidate_cache(code).await;
        Ok(saved)
    }

    pub async fn delete_type(&self, id: Uuid) -> Result<(), AppError> {
        let old = self.find_type_by_id(id).await?;
        sqlx::query("DELETE FROM dict_types WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除字典类型失败: {e}")))?;
        self.invalidate_cache(&old.code).await;
        Ok(())
    }

    /// 按 ID 查询字典类型
    ///
    /// v0.13.0 起为 `pub`：删除入口需要在**删之前**查一次 `code`，
    /// 让审计摘要能记下"删的是哪个字典"而不是只留一个 UUID。
    pub async fn find_type_by_id(&self, id: Uuid) -> Result<DictType, AppError> {
        sqlx::query_as::<_, DictType>("SELECT * FROM dict_types WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("字典类型不存在".into()))
    }

    // ── 字典项 ──────────────────────────────────────────

    pub async fn list_items(&self, type_id: Uuid) -> Result<Vec<DictItem>, AppError> {
        sqlx::query_as::<_, DictItem>(
            "SELECT * FROM dict_items WHERE dict_type_id = $1 ORDER BY sort_order ASC",
        )
        .bind(type_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询字典项失败: {e}")))
    }

    /// 列出**对外可用**的字典项：跳过 `status='disabled'` 的项
    ///
    /// 字典管理页需要看到禁用项（否则管理员没法把它们改回来），
    /// 因此 `list_items` 不过滤；读取端点必须过滤——管理页看得到、
    /// 业务页面看不到，才是"禁用"这个开关真正生效的样子。
    pub async fn list_enabled_items(&self, type_id: Uuid) -> Result<Vec<DictItem>, AppError> {
        sqlx::query_as::<_, DictItem>(
            "SELECT * FROM dict_items WHERE dict_type_id = $1 AND status = 'enabled' ORDER BY sort_order ASC",
        )
        .bind(type_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("查询字典项失败: {e}")))
    }

    /// 按 ID 查询字典项
    ///
    /// v0.13.0 起为 `pub`：删除入口需要在**删之前**查一次 `label`/`value`，
    /// 让审计摘要能记下"删的是哪个字典项"而不是只留一个 UUID。
    pub async fn find_item_by_id(&self, id: Uuid) -> Result<DictItem, AppError> {
        sqlx::query_as::<_, DictItem>("SELECT * FROM dict_items WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("字典项不存在".into()))
    }

    pub async fn create_item(&self, item: &DictItem) -> Result<DictItem, AppError> {
        // 与 update_item 同一条规则：禁用项不能当默认项，
        // 否则读取端点过滤掉它之后，"默认"就指向一个不存在的东西
        if item.is_default && item.status != "enabled" {
            return Err(AppError::BadRequest(
                "已禁用的字典项不能设为默认项，请先启用它".into(),
            ));
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        // 同一字典只允许一个默认项。设置新的默认项前先把旧的取消，
        // 否则"默认"这个词就没有意义（实测可同时存在任意多个）。
        //
        // 与 INSERT 放同一个事务：分两次写的话，并发请求会各自看到
        // "现在还没有默认项"，最后写两个，DB 唯一索引直接报错。
        if item.is_default {
            sqlx::query(
                "UPDATE dict_items SET is_default = FALSE WHERE dict_type_id = $1 AND is_default",
            )
            .bind(item.dict_type_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清除旧默认项失败: {e}")))?;
        }
        let saved = sqlx::query_as::<_, DictItem>(
            "INSERT INTO dict_items (id, dict_type_id, label, value, sort_order, status, is_default, color) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *",
        )
        .bind(item.id).bind(item.dict_type_id).bind(&item.label).bind(&item.value)
        .bind(item.sort_order).bind(&item.status).bind(item.is_default).bind(&item.color)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            let fallback = format!("创建字典项失败: {e}");
            map_dict_item_write_violation(e, fallback)
        })?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        self.invalidate_cache_for_type(item.dict_type_id).await;
        Ok(saved)
    }

    pub async fn update_item(
        &self,
        id: Uuid,
        fields: &crate::model::CreateDictItemRequest,
    ) -> Result<DictItem, AppError> {
        let old = sqlx::query_as::<_, DictItem>("SELECT * FROM dict_items WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("字典项不存在".into()))?;
        let label = &fields.label;
        let value = &fields.value;
        let sort = fields.sort_order.unwrap_or(old.sort_order);
        let status = fields.status.as_deref().unwrap_or(&old.status);
        let color = fields.color.as_deref().or(old.color.as_deref());

        // 禁用的项不能是默认项：读取端点按 status 过滤掉禁用项，
        // 于是"禁用 + 默认"= 一个谁都看不见的默认项——又是一个说了不算的开关。
        // 请求里显式要 `is_default=true` 时直接说清楚，不静默改写管理员的输入；
        // 没显式提（`None`）时按"禁用它就不该再当默认"处理，静默清除是符合意图的。
        let def = match fields.is_default {
            Some(true) if status != "enabled" => {
                return Err(AppError::BadRequest(
                    "已禁用的字典项不能设为默认项，请先启用它".into(),
                ));
            }
            Some(v) => v,
            None => status == "enabled" && old.is_default,
        };

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        if def {
            // 排除自身：否则会先把自己的 is_default 清成 FALSE
            sqlx::query("UPDATE dict_items SET is_default = FALSE WHERE dict_type_id = $1 AND is_default AND id <> $2")
                .bind(old.dict_type_id)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| AppError::InternalServerError(format!("清除旧默认项失败: {e}")))?;
        }
        let saved = sqlx::query_as::<_, DictItem>(
            "UPDATE dict_items SET label=$2,value=$3,sort_order=$4,status=$5,is_default=$6,color=$7 WHERE id=$1 RETURNING *",
        )
        .bind(id).bind(label).bind(value).bind(sort).bind(status).bind(def).bind(color)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            let fallback = format!("更新字典项失败: {e}");
            map_dict_item_write_violation(e, fallback)
        })?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        self.invalidate_cache_for_type(old.dict_type_id).await;
        Ok(saved)
    }

    pub async fn delete_item(&self, id: Uuid) -> Result<(), AppError> {
        let old = sqlx::query_as::<_, DictItem>("SELECT * FROM dict_items WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("字典项不存在".into()))?;

        sqlx::query("DELETE FROM dict_items WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除字典项失败: {e}")))?;
        self.invalidate_cache_for_type(old.dict_type_id).await;
        Ok(())
    }

    // ── 缓存与批量查询 ──────────────────────────────────

    /// 根据编码获取字典（先查 Redis，再查 DB）
    ///
    /// **只返回对外可用的数据**：`status='disabled'` 的字典类型与字典项都不返回。
    /// 这是"禁用"这个开关真正生效的地方——字典管理页仍看得到禁用项（`list_items`
    /// 不过滤，管理员要能改回来），但业务页面读不到。
    pub async fn get_dict_by_code(&self, code: &str) -> Result<Vec<DictItemResponse>, AppError> {
        // 尝试从 Redis 读取
        if let Some(ref redis) = self.redis {
            let cache_key = format!("{}{}", DICT_CACHE_PREFIX, code);
            if let Ok(Some(data)) = redis.get_string(&cache_key).await {
                if let Ok(items) = serde_json::from_str::<Vec<DictItemResponse>>(&data) {
                    return Ok(items);
                }
            }
        }
        // 回源到 DB
        let type_opt = self.find_type_by_code(code).await?;
        if let Some(t) = type_opt {
            // 类型被禁用 → 整份字典不可用。注意**不写缓存**：
            // 空结果一旦被缓存住，管理员重新启用类型后还得等 TTL 到期才恢复，
            // 而"我刚点了启用，怎么还没生效"正是这个开关最该避免的观感。
            if t.status != "enabled" {
                return Ok(vec![]);
            }
            let items = self.list_enabled_items(t.id).await?;
            let resp: Vec<DictItemResponse> = items
                .iter()
                .map(|i| DictItemResponse {
                    id: i.id,
                    label: i.label.clone(),
                    value: i.value.clone(),
                    sort_order: i.sort_order,
                    status: i.status.clone(),
                    is_default: i.is_default,
                    color: i.color.clone(),
                })
                .collect();
            // 写入 Redis 缓存
            if let Some(ref redis) = self.redis {
                let cache_key = format!("{}{}", DICT_CACHE_PREFIX, code);
                if let Ok(json) = serde_json::to_string(&resp) {
                    let _ = redis.set_string(&cache_key, &json, 3600).await;
                }
            }
            return Ok(resp);
        }
        Ok(vec![])
    }

    /// 查询所有字典类型（含项）
    pub async fn list_all_with_items(&self) -> Result<Vec<DictTypeWithItems>, AppError> {
        let types = self.list_types().await?;
        let mut result = vec![];
        for t in types {
            let items = self.list_items(t.id).await?;
            let item_resp: Vec<DictItemResponse> = items
                .iter()
                .map(|i| DictItemResponse {
                    id: i.id,
                    label: i.label.clone(),
                    value: i.value.clone(),
                    sort_order: i.sort_order,
                    status: i.status.clone(),
                    is_default: i.is_default,
                    color: i.color.clone(),
                })
                .collect();
            result.push(DictTypeWithItems {
                id: t.id,
                code: t.code,
                name: t.name,
                description: t.description,
                status: t.status,
                sort_order: t.sort_order,
                items: item_resp,
            });
        }
        Ok(result)
    }

    /// 使指定字典编码的缓存失效
    ///
    /// 缓存失效失败不应让写操作失败，但必须可观测，
    /// 否则用户会看到最长 1 小时的陈旧字典数据。
    async fn invalidate_cache(&self, code: &str) {
        if let Some(ref redis) = self.redis {
            let cache_key = format!("{}{}", DICT_CACHE_PREFIX, code);
            if let Err(e) = redis.delete_key(&cache_key).await {
                tracing::warn!("字典缓存失效失败 (key={cache_key}): {e}");
            }
        }
    }

    /// 按字典类型 ID 使其编码对应缓存失效
    async fn invalidate_cache_for_type(&self, type_id: Uuid) {
        match self.find_type_by_id(type_id).await {
            Ok(t) => self.invalidate_cache(&t.code).await,
            Err(e) => tracing::warn!("字典缓存失效跳过（类型 {type_id} 查询失败）: {e}"),
        }
    }

    /// 清空**全部**字典缓存，返回实际删除的键数
    ///
    /// 这是"刷新缓存"真正需要的那一步。写路径的 `invalidate_cache` 只覆盖
    /// 自己那几个键，一旦它失败（Redis 抖动）就没有任何补救手段了——
    /// 而管理员唯一能点的按钮此前只是把缓存读一遍再原样写回。
    pub async fn clear_all_dict_cache(&self) -> Result<u64, AppError> {
        match &self.redis {
            Some(r) => r.delete_by_prefix(DICT_CACHE_PREFIX).await,
            // 没有 Redis 客户端就意味着根本没有缓存。如实报 0，
            // 让界面能区分"本来就没有缓存"和"清掉了 N 个键"。
            None => Ok(0),
        }
    }
}
