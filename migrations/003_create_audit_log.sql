-- 操作日志表
-- 记录用户操作、请求参数、响应结果、IP、时间
CREATE TABLE IF NOT EXISTS audit_logs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID,
    username    VARCHAR(50),
    action      VARCHAR(100)  NOT NULL,
    method      VARCHAR(10)   NOT NULL,
    path        VARCHAR(500)  NOT NULL,
    params      TEXT,
    result      TEXT,
    status_code INTEGER,
    client_ip   VARCHAR(50),
    duration_ms INTEGER,
    created_at  TIMESTAMPTZ   NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_id ON audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action  ON audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created  ON audit_logs(created_at DESC);
