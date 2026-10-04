-- v0.23.0：开放注册开关
--
-- 背景：`/api/auth/register` 挂在 `public_routes` 上**无条件开放**，
-- 且注册成功即自动分配 `user` 角色（`service/auth.rs`）。
-- 在 v0.22.0 之前，一个部署到公网的实例**没有任何办法关掉这件事**——
-- grep `allow_register|registration_enabled` 全仓零命中。
-- 这是当时唯一一条"部署出去就一直在敞开"的缺口。
--
-- ── 为什么默认值是 `true`（保持开放）────────────────────────
-- 本参数出现之前，注册就是无条件开放的，这是**既成事实**而非疏忽。
-- 把默认值设成 `false` 会让一次常规发版突然关掉所有存量部署的注册入口，
-- 包括那些**故意**开放注册的内部系统。收紧的意图必须由管理员显式表达。
--
-- ── 为什么它属于"参数"而不是环境变量 ────────────────────────
-- 与口令策略同一条理由：这是**业务策略**不是部署形态。
-- 管理员应当能在界面上关掉它、能看到是谁关的、能随时改回来，
-- 而不必改 compose 文件走一次发版。

INSERT INTO system_settings (key, value) VALUES
    ('security.registration.enabled', 'true')
ON CONFLICT (key) DO NOTHING;

COMMENT ON COLUMN system_settings.value IS
    '以文本存储，实际类型由 SETTING_DEFS 的 SettingType 决定';

-- 注：本迁移**只加种子行，不改表结构**。
-- `system_settings` 的 key 是主键且由 `SETTING_DEFS` 定义，
-- 新增参数因此不需要 DDL——这正是 v0.22.0 选择"文本 key + 单表"的收益。
