//! 两步验证数据访问层（Repository）
//!
//! 涉及两张表：`users` 上的两列（密钥密文 + 生效时间）与
//! `user_two_factor_recovery`（恢复码摘要）。
//!
//! 刻意不建独立的 `user_two_factor` 表：2FA 的全部状态就是"这个用户有没有
//! 一把密钥"，拆表只会让"用户与密钥"变成逻辑上 1:1 但物理分离，
//! 换来一次 JOIN 和一处可能不一致的中间状态。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::AppError;
use sqlx::PgPool;

/// 用户的 2FA 状态
#[derive(Debug, Clone)]
pub struct TwoFactorState {
    /// 加密后的 TOTP 密钥；`None` 表示未绑定
    pub secret_enc: Option<Vec<u8>>,
    /// 生效时间；`None` 表示未启用
    pub enabled_at: Option<DateTime<Utc>>,
}

impl TwoFactorState {
    /// 是否处于"登录必须过第二道因子"的状态
    ///
    /// 判定要求两列都在：密钥存在但未确认过验证码的中间态
    /// （用户扫码了但没输对码）**不算启用**，此时登录不拦人——
    /// 否则用户会把自己锁在一个从未完成的绑定流程里。
    pub fn is_enabled(&self) -> bool {
        self.enabled_at.is_some() && self.secret_enc.is_some()
    }
}

/// `users` 上那两列的查询结果
///
/// 单独命名而不是内联元组：`(Option<Vec<u8>>, Option<DateTime>)` 这种
/// 嵌套 Option 的元组在调用处读不出"哪个 None 代表没绑定"，
/// 字段名把这件事写进类型里。
#[derive(sqlx::FromRow)]
struct TwoFactorColumns {
    totp_secret_enc: Option<Vec<u8>>,
    totp_enabled_at: Option<DateTime<Utc>>,
}

/// 两步验证数据访问层
#[derive(Debug, Clone)]
pub struct TwoFactorRepository {
    pool: PgPool,
}

impl TwoFactorRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 读用户的 2FA 状态
    ///
    /// 用户不存在时返回全空状态而不是报错：这个方法的调用方是登录流程，
    /// 它在更上游已经确认过用户存在，这里再抛 404 只会让登录错误多一层无意义的分支。
    pub async fn find_state(&self, user_id: Uuid) -> Result<TwoFactorState, AppError> {
        let row: Option<TwoFactorColumns> =
            sqlx::query_as("SELECT totp_secret_enc, totp_enabled_at FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AppError::InternalServerError(format!("读取 2FA 状态失败: {e}")))?;
        Ok(match row {
            Some(cols) => TwoFactorState {
                secret_enc: cols.totp_secret_enc,
                enabled_at: cols.totp_enabled_at,
            },
            None => TwoFactorState {
                secret_enc: None,
                enabled_at: None,
            },
        })
    }

    /// 写入密钥密文并**同时**置为生效
    ///
    /// 绑定流程只有"扫码 → 输对码 → 一次性落库"这一步，
    /// 不保留"密钥已写但未生效"的持久态（那会让用户重开页面后
    /// 既不能重新绑定、又要带着一把废密钥）。
    pub async fn enable(&self, user_id: Uuid, secret_enc: Vec<u8>) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET totp_secret_enc = $2, totp_enabled_at = NOW() WHERE id = $1")
            .bind(user_id)
            .bind(secret_enc)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::InternalServerError(format!("启用两步验证失败: {e}")))?;
        Ok(())
    }

    /// 关闭 2FA：清空密钥与生效时间
    ///
    /// 恢复码由迁移 019 的触发器级联清理，不在这里手写 DELETE。
    pub async fn disable(&self, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE users SET totp_secret_enc = NULL, totp_enabled_at = NULL WHERE id = $1",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("关闭两步验证失败: {e}")))?;
        Ok(())
    }

    /// 剩余可用恢复码数量
    pub async fn count_unused_recovery_codes(&self, user_id: Uuid) -> Result<i64, AppError> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM user_two_factor_recovery \
             WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("统计恢复码失败: {e}")))?;
        Ok(count)
    }

    /// 用一批新恢复码替换该用户全部未使用的恢复码
    ///
    /// 整批替换而非增量追加：换一批的语义就是"之前那批全部作废"。
    /// 删除与插入必须在同一事务里，否则中途失败会留下"删掉了旧的、
    /// 新的没写进去"的状态——用户既没有旧码可用、也没有新码。
    pub async fn replace_recovery_codes(
        &self,
        user_id: Uuid,
        hashes: &[String],
    ) -> Result<(), AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(format!("开启事务失败: {e}")))?;
        sqlx::query("DELETE FROM user_two_factor_recovery WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("清理旧恢复码失败: {e}")))?;
        for hash in hashes {
            sqlx::query(
                "INSERT INTO user_two_factor_recovery (user_id, code_hash) VALUES ($1, $2)",
            )
            .bind(user_id)
            .bind(hash)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::InternalServerError(format!("写入恢复码失败: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(format!("提交事务失败: {e}")))?;
        Ok(())
    }

    /// 核销一个恢复码
    ///
    /// 用 `used_at IS NULL` 作为更新条件并返回受影响行数：
    /// **同一恢复码并发提交两次时只有一次能拿到 1**，另一次拿到 0 并被判失败。
    /// 靠应用层"先查后写"会有这个竞态——而恢复码正是最不该能被并发重放的东西。
    pub async fn consume_recovery_code(&self, user_id: Uuid, hash: &str) -> Result<bool, AppError> {
        let affected = sqlx::query(
            "UPDATE user_two_factor_recovery SET used_at = NOW() \
             WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
        )
        .bind(user_id)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::InternalServerError(format!("核销恢复码失败: {e}")))?
        .rows_affected();
        Ok(affected == 1)
    }
}
