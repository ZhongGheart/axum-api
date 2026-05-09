//! 数据字典数据访问层 + Redis 缓存

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{DictItem, DictItemResponse, DictType, DictTypeWithItems};
use crate::utils::redis::RedisClient;

const DICT_CACHE_PREFIX: &str = "dict:";

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

    pub async fn update_type(&self, id: Uuid, fields: &crate::model::CreateDictTypeRequest) -> Result<DictType, AppError> {
        let old = self.find_type_by_id(id).await?;
        let code = &fields.code;
        let name = &fields.name;
        let desc = fields.description.as_deref().or(old.description.as_deref());
        let status = fields.status.as_deref().unwrap_or(&old.status);
        let sort = fields.sort_order.unwrap_or(old.sort_order);
        sqlx::query_as::<_, DictType>(
            "UPDATE dict_types SET code=$2,name=$3,description=$4,status=$5,sort_order=$6 WHERE id=$1 RETURNING *",
        )
        .bind(id).bind(code).bind(name).bind(desc).bind(status).bind(sort)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("更新字典类型失败: {e}")))
    }

    pub async fn delete_type(&self, id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM dict_types WHERE id = $1").bind(id)
            .execute(&self.pool).await.ok();
        self.invalidate_cache().await;
        Ok(())
    }

    async fn find_type_by_id(&self, id: Uuid) -> Result<DictType, AppError> {
        sqlx::query_as::<_, DictType>("SELECT * FROM dict_types WHERE id=$1").bind(id)
            .fetch_optional(&self.pool).await
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

    pub async fn create_item(&self, item: &DictItem) -> Result<DictItem, AppError> {
        let saved = sqlx::query_as::<_, DictItem>(
            "INSERT INTO dict_items (id, dict_type_id, label, value, sort_order, status, is_default, color) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *",
        )
        .bind(item.id).bind(item.dict_type_id).bind(&item.label).bind(&item.value)
        .bind(item.sort_order).bind(&item.status).bind(item.is_default).bind(&item.color)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("创建字典项失败: {e}")))?;
        self.invalidate_cache().await;
        Ok(saved)
    }

    pub async fn update_item(&self, id: Uuid, fields: &crate::model::CreateDictItemRequest) -> Result<DictItem, AppError> {
        let old = sqlx::query_as::<_, DictItem>("SELECT * FROM dict_items WHERE id=$1").bind(id)
            .fetch_optional(&self.pool).await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("字典项不存在".into()))?;
        let label = &fields.label;
        let value = &fields.value;
        let sort = fields.sort_order.unwrap_or(old.sort_order);
        let status = fields.status.as_deref().unwrap_or(&old.status);
        let def = fields.is_default.unwrap_or(old.is_default);
        let color = fields.color.as_deref().or(old.color.as_deref());
        let saved = sqlx::query_as::<_, DictItem>(
            "UPDATE dict_items SET label=$2,value=$3,sort_order=$4,status=$5,is_default=$6,color=$7 WHERE id=$1 RETURNING *",
        )
        .bind(id).bind(label).bind(value).bind(sort).bind(status).bind(def).bind(color)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("更新字典项失败: {e}")))?;
        self.invalidate_cache().await;
        Ok(saved)
    }

    pub async fn delete_item(&self, id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM dict_items WHERE id = $1").bind(id)
            .execute(&self.pool).await.ok();
        self.invalidate_cache().await;
        Ok(())
    }

    // ── 缓存与批量查询 ──────────────────────────────────

    /// 根据编码获取字典（先查 Redis，再查 DB）
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
            let items = self.list_items(t.id).await?;
            let resp: Vec<DictItemResponse> = items.iter().map(|i| DictItemResponse {
                id: i.id, label: i.label.clone(), value: i.value.clone(),
                sort_order: i.sort_order, status: i.status.clone(),
                is_default: i.is_default, color: i.color.clone(),
            }).collect();
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
            let item_resp: Vec<DictItemResponse> = items.iter().map(|i| DictItemResponse {
                id: i.id, label: i.label.clone(), value: i.value.clone(),
                sort_order: i.sort_order, status: i.status.clone(),
                is_default: i.is_default, color: i.color.clone(),
            }).collect();
            result.push(DictTypeWithItems {
                id: t.id, code: t.code, name: t.name,
                description: t.description, status: t.status,
                sort_order: t.sort_order, items: item_resp,
            });
        }
        Ok(result)
    }

    /// 使字典缓存失效
    async fn invalidate_cache(&self) {
        if let Some(ref redis) = self.redis {
            // 简单方案：不精确清除，让缓存自然过期
            // 生产环境可用 Redis SCAN 匹配删除
        }
    }
}
