-- v0.22.0：系统参数配置表 + 口令策略可配
--
-- 背景：此前所有可调项都在 `config/mod.rs` 从环境变量读，**改一个登录失败窗口
-- 要重启进程**。更糟的是环境变量在容器编排里是启动时固化的，
-- 于是"管理员想收紧口令策略"这件事在产品层面根本不存在。
--
-- ── 为什么是"表"而不是继续扩环境变量 ────────────────────────
-- 环境变量适合**部署形态**（端口、连接串、密钥），不适合**业务策略**
-- （口令多长、几天过期）。后者要能在管理界面上改、能审计、能回滚。
-- 混在一起的结果就是：策略被当成部署配置塞进 compose 文件，
-- 改一次要走一次发版流程。
--
-- ── 为什么 key 用文本而不是自增 id ──────────────────────────
-- 参数**由代码里的 `model::setting::SETTING_DEFS` 定义**（单一数据源），
-- DB 只是它的持久化。管理员不能凭空造参数，只能改已定义的参数的取值。
-- 用 key 作主键才能让"未定义的 key"在 DB 层就没有落点。

CREATE TABLE IF NOT EXISTS system_settings (
    -- 参数名，与 `model::setting::SETTING_DEFS` 的 `key` 逐字对应
    key         VARCHAR(64) PRIMARY KEY,
    -- 取值一律以**文本**存储。
    --
    -- 刻意不按类型分列：参数种类少、分列会让"新增一个参数"变成一次迁移。
    -- 类型信息由代码侧的 `SettingType` 负责，写入时校验、读取时解析，
    -- 解析失败一律回落到 `default` 而不是报错（见 `service/setting.rs`）。
    value       TEXT        NOT NULL,
    -- 最近一次修改者；NULL 表示种子写入（没有"人"）
    updated_by  UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE system_settings IS
    '系统参数。参数名与取值范围由 src/model/setting.rs 的 SETTING_DEFS 定义，管理员只能改已定义参数的取值，不能新增参数。';
COMMENT ON COLUMN system_settings.key IS
    '参数名，与 SETTING_DEFS 的 key 逐字对应；DB 层不做外键，因为定义在代码里';
COMMENT ON COLUMN system_settings.value IS
    '以文本存储，实际类型由 SETTING_DEFS 的 SettingType 决定';

-- updated_by 不加外键约束：故意允许保留已删除用户所写的参数，
-- 否则删一个用户会让"谁改的这个参数"变成孤儿记录进而整行被挡。
-- 审计轨迹的权威来源是 `audit_logs`，这一列只是方便界面上显示一个名字。

-- ── 口令策略的四个参数 ───────────────────────────────────────
--
-- 默认值必须与 v0.21.0 及此前的**硬编码常量完全一致**
-- （PASSWORD_MIN_LEN=8 / MAX_LEN=128 / 至少 2 类字符 / 不过期）。
-- 抬门槛的当天把所有存量用户锁在门外，是本仓反复确认过的教训
-- （见迁移 010 的注释：默认值设错等于叠加第二次强制登出冲击）。
INSERT INTO system_settings (key, value) VALUES
    ('security.password.min_length',            '8'),
    ('security.password.max_length',            '128'),
    ('security.password.min_char_classes',      '2'),
    ('security.password.expiry_days',           '0'),
    ('security.login.max_failures',             '10'),
    ('security.login.failure_window_seconds',   '300'),
    ('security.password.require_mixed_case',    'false')
ON CONFLICT (key) DO NOTHING;

-- 刻意**没有**"口令历史"参数（禁止复用最近 N 个旧口令）。
-- 它既不属于复杂度也不属于过期策略，要真正生效还得新增一张
-- 口令历史表 + 每条写入路径（注册/自助改密/管理员重置/CSV 导入）都去写它。
-- 在一个已经要动登录与改密路径的版本里塞进来，风险与收益不成比例。
-- 本仓的教训（见 v0.16.0）正是：**一个没有效果的开关比没有开关更糟**，
-- 管理员会以为策略已经生效而据此放宽其他控制。

COMMENT ON COLUMN system_settings.updated_by IS
    '最近修改该参数的用户 ID；NULL 表示种子写入。仅供界面显示，审计以 audit_logs 为准';

-- ── users.password_changed_at ─────────────────────────────────
--
-- 口令过期策略需要知道"这个口令是什么时候设的"。
-- `users.updated_at` **不能**拿来顶替：v0.20.0 的自助改资料、
-- 管理员改显示名都会刷新它，于是"改了个头像"会被算成"刚换过口令"，
-- 过期策略被无限推迟——一个安全策略被另一个无关功能静默关掉。
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS password_changed_at TIMESTAMPTZ;

COMMENT ON COLUMN users.password_changed_at IS
    '口令最近一次被设置的时刻。存量行为 NULL，表示"无法判断"，此时不强制过期（见下方回填）';

-- 存量行回填成 created_at 而不是留 NULL：
-- 留 NULL 就得在应用层写"NULL 视为不过期"的分支，而那个分支会永久存在。
-- 回填成 created_at 后，**逻辑完全等价**（都是"从很久以前算起，都该过期了"），
-- 却不需要任何特判。代价是：一旦管理员把 expiry_days 从 0 调成非 0，
-- 所有存量用户会在下一次登录时被要求改密。
--
-- 这个副作用是**可接受的且必须明说**：默认 expiry_days=0（不过期），
-- 所以默认部署下没有任何人受影响；管理员主动开启过期策略时，
-- "存量口令都算过期"恰恰是这条策略应有的语义——
-- 强制全体换一次口令正是启用它的目的。
-- 真正需要小心的是**不要**反过来设成 NOW()：那会让所有存量用户
-- 在管理员开启策略后又白白多活一个完整周期，策略形同虚设。
UPDATE users
SET password_changed_at = created_at
WHERE password_changed_at IS NULL;

-- 改过口令的用户必须落时间戳，否则过期策略对他们永远不生效。
-- 放在迁移里做一次存量回填是安全的：新写入路径（注册/改密/重置）
-- 会在同一条语句里显式写入该列。
--
-- 注：这里**不**用 `now()` 全表刷——那会让存量用户在开启过期策略后
-- 再多活一个周期，正是上面说的"策略形同虚设"。
