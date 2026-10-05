-- v0.26.0：审计明细结构化查询（D1）
--
-- 背景：`utils/audit.rs` 的 `diff_summary` / `permission_change` 已接进全部写 handler，
-- 但**只产出人类可读的 `result` 字符串**。于是审计只能按 `action` 模糊筛，
-- 答不出"谁改过 role:3 的权限"——而这恰恰是审计最常被问到的问题。
--
-- ── 为什么用子表而不是给 audit_logs 加列 ──────────────────
-- 直觉方案是加 `target_type` / `target_id` / `change_type` 三列。**这个方案不成立**，
-- 因为它假设"一次请求只碰一个对象"，而实际不是：
--
-- - `POST /api/admin/users/batch-delete` 一次碰 N 个用户
-- - `DELETE /api/admin/roles/{id}` 连带撤销该角色的 N 个权限码（指向 N 个菜单）
--
-- 单列在这两种场景下只有两个选择，都不可接受：
-- 存第一个 target → 其余静默丢失，"这次操作碰过哪些对象"答不全；
-- 存成逗号拼接串 → `target_id` 是 UUID，拼接后按 id 筛只能 `LIKE '%...%'`，
--   索引彻底失效，等于没有结构化。
-- 这与 `repository/audit_log.rs` 里 `fetch_for_export` 的注释是同一条道理：
-- **静默截断比报错更糟**，而"只记第一个"就是静默截断。
--
-- 子表一行一个 target，上述场景一条不丢，且 `(target_type, target_id)`
-- 是干净的两列等值查询，索引能正常用。筛选走 `EXISTS` 子查询。
--
-- ── 为什么 target_id 不加外键 ──────────────────────────
-- 被指向的行**经常已经不存在了**：`DELETE /api/admin/roles/{id}` 的 target 就是
-- 那个刚被删掉的角色。加外键会让这类审计行无法插入，或在删除时被级联清掉——
-- 而"删掉的是什么"恰恰是删除类审计**唯一**的信息来源。
-- 审计要能指向已经消失的东西，所以这里只存裸 UUID，不建立引用完整性。

CREATE TABLE IF NOT EXISTS audit_log_targets (
    -- 代理主键。**不能拿 (audit_log_id, target_type, target_id, change_type) 当主键**：
    -- `target_id` 允许为 NULL（见下），而主键列不允许 NULL。
    -- 去重改由下面的唯一索引承担，语义等价。
    id           BIGSERIAL   PRIMARY KEY,
    -- 所属审计行。级联删除是**正确**的：日志行没了，它的 target 引用也没有意义
    audit_log_id UUID NOT NULL REFERENCES audit_logs(id) ON DELETE CASCADE,
    -- 资源种类：`user` / `role` / `menu` / `dict_type` / `dict_item` /
    --           `department` / `setting` / `user_two_factor`
    target_type  VARCHAR(32)  NOT NULL,
    -- 被操作对象的 ID。**故意不加外键**，理由见文件头
    --
    -- 可空：系统参数（`security.password.min_length`）这类资源**没有 UUID**，
    -- 主键就是字符串。硬塞一个假 UUID 进去会让"按参数名查审计"彻底不可能，
    -- 而这恰好是排查口令策略被谁改掉时唯一的入口。
    -- 因此另开 `target_key` 承载字符串主键，两者至少有一个非空（见 CHECK）。
    target_id    UUID,
    -- 字符串主键的资源的键。当前只有 `target_type = 'setting'` 会用到
    target_key   VARCHAR(200),
    -- 变更类型：`create` / `update` / `delete` / `grant` / `revoke` /
    --           `enable` / `disable` / `status` / `revoke_session` / `login`
    change_type  VARCHAR(24)  NOT NULL,
    -- 冗余存一份当时的名字：目标行删掉后仍能答出"改的是什么"
    target_label VARCHAR(200),
    -- 至少要能指向一个东西：两者都空的行等于没记录对象，是纯噪声
    CONSTRAINT audit_log_targets_identity CHECK (
        target_id IS NOT NULL OR target_key IS NOT NULL
    )
);

-- 去重。同一行里同一个对象的同一类变更只留一条。
-- 用表达式索引把 `target_id` 与 `target_key` 两套标识合成一列参与唯一性，
-- 否则两者都为 NULL 时唯一约束失效（NULL 不等于 NULL），
-- 重复行就会插进来——于是"这个按钮被谁动过"出现重复条目。
CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_log_targets_unique
    ON audit_log_targets(
        audit_log_id, target_type, change_type, COALESCE(target_id::text, target_key)
    );

-- 筛选 `target_type = ? AND target_id = ?` 的主力索引。
CREATE INDEX IF NOT EXISTS idx_audit_log_targets_lookup
    ON audit_log_targets(target_type, target_id);

-- 按参数名查审计（`target_type = 'setting' AND target_key = ?`）
CREATE INDEX IF NOT EXISTS idx_audit_log_targets_key_lookup
    ON audit_log_targets(target_type, target_key)
    WHERE target_key IS NOT NULL;

-- 按审计行反查它的全部 target（渲染列表页"涉及对象"列时用）
CREATE INDEX IF NOT EXISTS idx_audit_log_targets_by_log
    ON audit_log_targets(audit_log_id);

COMMENT ON TABLE audit_log_targets IS
    '审计明细的结构化对象引用：一条 audit_logs 可对应 N 行。';

COMMENT ON COLUMN audit_log_targets.target_id IS
    '被操作对象的 ID，**故意不加外键**——删除类审计指向的行已不存在，加外键会让审计无法落库或被级联清掉。';

-- ── 为什么不做存量回填 ──────────────────────────────────
-- ROADMAP 原写"从 `result` 字符串解析回填"。**实测后决定不做**，理由是回填必然出错：
--
-- 1. 同一段文本里的 UUID 不总是句尾那一个。
--    `为用户 "X" 追加角色 "Y"（<uuid>）` 里的 UUID 是**用户**的，不是"Y"这个角色的。
--    按"取最后一个 UUID"回填会把它记成 role——张冠李戴，且错得毫无痕迹。
-- 2. 相当一部分摘要根本不含 UUID。实测 `批量导入用户：共 6 行，成功 2，失败 4；…`
--    只列用户名，`自助吊销单个会话（d6d2c2a2…）` 只留 8 位前缀，回填无源可取。
-- 3. `change_type` 无法从中文文本可靠反解：`更新用户` / `停用用户` / `重置用户口令`
--    三种语义在文本层面难以区分，硬猜出来的值会被当成事实引用。
--
-- 错误的结构化数据比没有结构化数据**更危险**：它看起来可信，会被直接拿来回答
-- "谁动过它"，而答案是错的。宁可让历史记录老老实实留在文本形态、
-- 界面上如实说明"该记录早于结构化上线，无对象标注"，也不往审计里写猜出来的对象。
