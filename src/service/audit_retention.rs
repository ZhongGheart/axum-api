//! 审计日志保留策略（后台清理任务）
//!
//! `audit_logs` 每来一个已认证请求就插一行，且没有分区、没有清理，
//! 于是它会成为随时间单调增长的表——最终既拖慢查询，也把磁盘吃满。
//! 本模块按配置的保留天数定期删除过期行。
//!
//! 两个刻意的取舍：
//!
//! - **删除必须留痕**：审计数据被静默删除是不可接受的。
//!   出了事没人知道日志是什么时候没的，因此每次删除都打 `info`，
//!   记录截止时间点与删除行数
//! - **不做跨副本选主**：多副本会各自跑清理。
//!   由于删除按批次提交且走同一条索引，重复劳动只是徒增锁竞争，
//!   不会产生错误结果；为此引入分布式锁不值得

use chrono::{Duration, Utc};
use sqlx::PgPool;

use crate::config::AuditLogConfig;
use crate::repository::audit_log::AuditLogRepository;

/// 启动清理任务；保留策略关闭（`retention_days = 0`）时返回 `None`
pub fn spawn_audit_retention(pool: PgPool, cfg: AuditLogConfig) -> Option<AuditRetentionTask> {
    if cfg.retention_days == 0 {
        tracing::info!("审计日志自动清理已关闭（AUDIT_LOG_RETENTION_DAYS=0），请自行安排清理");
        return None;
    }

    let repo = AuditLogRepository::new(pool);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let interval = std::time::Duration::from_secs(cfg.cleanup_interval_seconds.max(1));

    tracing::info!(
        保留天数 = cfg.retention_days,
        间隔秒 = cfg.cleanup_interval_seconds,
        批大小 = cfg.cleanup_batch_size,
        "审计日志保留策略已启用"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                // interval 的首次 tick 立即触发：启动即补一次，
                // 否则进程停机期间攒下的过期日志要等满一个间隔才被清
                _ = ticker.tick() => {
                    if let Err(e) = run_once(&repo, &cfg).await {
                        tracing::error!("审计日志清理失败: {e}");
                    }
                }
                _ = shutdown_rx.changed() => break,
            }
        }
    });

    Some(AuditRetentionTask {
        shutdown: shutdown_tx,
        handle: Some(handle),
    })
}

/// 执行一轮清理
///
/// **删了行就必须留痕**。删除动作本身要写进 `audit_log_purges`：
/// 只有 `tracing::info!` 的话，"日志为什么从某个时间点起就查不到了"
/// 这个问题管理员在界面上与接口上都答不出来，只能去翻进程 stdout。
/// 而 v0.13.0 之后 `result` 是"改了什么"的唯一副本，
/// 删掉的不只是流水，是复盘能力本身。
///
/// 留痕失败**不掩盖**删除已经发生的事实：`tracing::error!` 之后照常返回，
/// 让运维从服务日志里看到"清理成功了但没记上"这个组合异常。
async fn run_once(
    repo: &AuditLogRepository,
    cfg: &AuditLogConfig,
) -> Result<u64, crate::error::AppError> {
    let started = std::time::Instant::now();
    let cutoff = Utc::now() - Duration::days(i64::from(cfg.retention_days));
    let outcome = repo
        .delete_older_than(cutoff, cfg.cleanup_batch_size, cfg.cleanup_max_batches)
        .await?;

    if outcome.deleted == 0 {
        // 一行没删就不记：每轮都记只会把表变成噪声，
        // 而"有清理发生过"才是需要被看见的事实
        return Ok(0);
    }

    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    if let Err(e) = repo.record_purge(cutoff, &outcome, duration_ms).await {
        // 删除已经提交，回滚不了；只能把异常暴露到服务日志
        tracing::error!(
            error = %e,
            截止时间 = %cutoff.to_rfc3339(),
            删除行数 = outcome.deleted,
            "审计日志清理已生效，但留痕失败——该轮删除无法在界面上查到"
        );
    }

    tracing::info!(
        截止时间 = %cutoff.to_rfc3339(),
        删除行数 = outcome.deleted,
        撞上限提前收手 = outcome.hit_batch_limit,
        耗时毫秒 = duration_ms,
        "已清理过期操作日志"
    );
    Ok(outcome.deleted)
}

/// 清理任务的句柄
pub struct AuditRetentionTask {
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AuditRetentionTask {
    /// 通知任务停止并等待其退出
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}
