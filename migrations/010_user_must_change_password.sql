-- v0.11.0：为 users 增加"首次登录强制改密"标记
--
-- **默认值必须是 FALSE**：存量用户补列后一律不受影响。
-- 这一点是 v0.11.0 的核心约束——`iat_ms` 升级已经让存量令牌作废过一次，
-- 若这里再默认 TRUE，等于叠加第二次强制登出冲击。
-- 只有管理员**新建**或**重置**的用户才会被置为 TRUE。

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS must_change_password BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN users.must_change_password IS
    '为 true 时，令牌被限制为只能调用改密/登出/me 接口，直到用户自助改密完成';
