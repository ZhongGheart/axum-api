-- v0.14.0：审计日志保留策略的清理动作自证
--
-- 背景：`service/audit_retention.rs` 每小时把 `created_at` 早于
-- `AUDIT_LOG_RETENTION_DAYS`（默认 90 天）的审计行**无条件删除**，
-- 而这件事此前只有一行 `tracing::info!`——进的是进程 stdout。
-- 于是"日志为什么少了"这个问题，管理员在界面上、接口上都答不出来，
-- 只能去翻服务器日志。而这恰恰是出事时最需要被回答的问题。
--
-- v0.13.0 之后更要紧：`result` 列成了"改了什么"的**唯一**存放处
-- （全库没有变更历史表），于是清理掉的不只是流水，而是复盘能力本身。
--
-- 为什么单独一张表，而不用 `audit_logs` 自己记：
-- 审计表记录自己的被删，会陷入"删这行要不要连带记录、
-- 记录的那行算不算过期"的递归。这张表与保留策略同寿命
-- （保留策略只删 `audit_logs`），因此永远不会自己把自己删掉。

CREATE TABLE IF NOT EXISTS audit_log_purges (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 本轮删掉的行的最后时刻下界：被删的都是 `created_at < cutoff_at` 的行。
    -- 有这个值才能回答"从哪一天起的数据已经没了"
    cutoff_at     TIMESTAMPTZ NOT NULL,
    deleted_rows  BIGINT      NOT NULL,
    ran_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    duration_ms   INTEGER,
    -- 因达到单轮批数上限而提前收手：此时 `cutoff_at` 之后的行可能也已过期
    -- 却仍在库里。"最老一条日志早于 cutoff_at"就是这种情况，
    -- 不记下来会把"还有更多过期数据没清"误报成"已经清干净了"
    hit_batch_limit BOOLEAN   NOT NULL DEFAULT FALSE
);

-- 只需要"最近一次"，按时间倒序取第一条；这个索引让它不必扫全表
CREATE INDEX IF NOT EXISTS idx_audit_log_purges_ran
    ON audit_log_purges(ran_at DESC);

COMMENT ON TABLE audit_log_purges IS
    '审计日志保留策略的每轮清理记录：让"日志被清掉了"这件事本身可查';
COMMENT ON COLUMN audit_log_purges.cutoff_at IS
    '本轮删除的行都早于该时刻';
COMMENT ON COLUMN audit_log_purges.hit_batch_limit IS
    '为 true 表示达到单轮批数上限提前收手，仍有过期行留在库中';
