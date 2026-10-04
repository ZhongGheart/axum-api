-- v0.23.0：账号安全闭环（C2 开放注册开关 + A2 并发会话上限）
--
-- 背景一：`/api/auth/register` 挂在 `public_routes` 上**无条件开放**，
-- 且注册成功即自动分配 `user` 角色（`service/auth.rs`）。
-- 在 v0.22.0 之前，一个部署到公网的实例**没有任何办法关掉这件事**：
-- grep `allow_register|registration_enabled` 全仓零命中。
-- 这是当时唯一一条"部署出去就一直在敞开"的缺口。
--
-- 背景二：同一账号可在任意多设备同时在线，
-- `max_session|concurrent|session_limit` 全仓零命中，
-- 管理员只能事后逐个吊销。
--
-- ── 两个默认值都是"保持原状" ────────────────────────────────
-- `security.registration.enabled` 默认 `true`：本参数出现之前注册就是无条件
-- 开放的，这是**既成事实**而非疏忽。默认 `false` 会让一次常规发版突然关掉
-- 所有存量部署的注册入口，包括**故意**开放注册的内部系统。
-- `security.session.max_concurrent` 默认 `0`（不限制）：同理，此前同一账号
-- 可在任意多设备同时在线，默认非 0 会让一次常规发版突然只允许有限设备登录。
-- 两处的收紧意图都必须由管理员显式表达。
--
-- ── 为什么它们属于"参数"而不是环境变量 ──────────────────────
-- 与口令策略同一条理由：这是**业务策略**不是部署形态。
-- 管理员应当能在界面上改、能看到是谁改的、能随时改回来，
-- 而不必改 compose 文件走一次发版。

INSERT INTO system_settings (key, value) VALUES
    ('security.registration.enabled', 'true'),
    ('security.session.max_concurrent', '0')
ON CONFLICT (key) DO NOTHING;

COMMENT ON COLUMN system_settings.value IS
    '以文本存储，实际类型由 SETTING_DEFS 的 SettingType 决定';

-- 注：本迁移**只加种子行，不改表结构**。
-- `system_settings` 的 key 是主键且由 `SETTING_DEFS` 定义，
-- 新增参数因此不需要 DDL——这正是 v0.22.0 选择"文本 key + 单表"的收益。
