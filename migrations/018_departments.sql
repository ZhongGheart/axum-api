-- v0.24.0：部门 / 组织树
--
-- 背景：`users` 与 `roles` 之间没有中间层，`dept|department|organization`
-- 在 src/migrations/frontend 三处 grep 均零命中。一个用户只能属于"全局"，
-- 无法表达"张三属于技术部-后端组"这种结构。
--
-- ── 为什么用自引用表而不是物化路径 / 嵌套集 ────────────────
-- 自引用表（`parent_id` 指向自己）是最简单且最不容易出错的树形结构：
-- - 物化路径（`path = /1/2/3/`）查询快但写入时要维护路径字符串，
--   移动子树需要更新整棵子树的路径，容易出错。
-- - 嵌套集（`left`/`right`）查询快但写入时要更新大量行的左右值，
--   移动子树的代价是 O(n)。
-- - 自引用表写入只改一行，查询用应用层 `build_tree`（与菜单树同一模式），
--   部门数量级在几十到几百，应用层构建完全够用。
--
-- ── 为什么 `parent_id` 用 `ON DELETE RESTRICT` ────────────────
-- 阻止删除有子部门的节点。服务层会先检查有没有子部门，有就拒绝并提示
-- "请先移动或删除子部门"。这比级联删除安全得多——级联删除可能误删
-- 大量数据，而"先处理子部门"是一个明确且可操作的错误提示。
--
-- ── 为什么 `users.dept_id` 用 `ON DELETE SET NULL` ─────────────
-- 删部门时用户变成"无部门"，而不是被级联删除。
-- 用户是核心数据，部门是组织结构；删一个部门不该删掉一群人。
-- 用户变成"无部门"后，管理员可以重新分配。

CREATE TABLE IF NOT EXISTS departments (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_id   UUID REFERENCES departments(id) ON DELETE RESTRICT,
    name        VARCHAR(100) NOT NULL,
    description TEXT,
    sort_order  INTEGER      NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE departments IS
    '部门 / 组织树。自引用表，parent_id 指向自己。ON DELETE RESTRICT 阻止删除有子部门的节点。';
COMMENT ON COLUMN departments.parent_id IS
    '父部门 ID；NULL 表示根部门。ON DELETE RESTRICT：有子部门时拒绝删除。';
COMMENT ON COLUMN departments.name IS
    '部门名称。同一父部门下唯一（由服务层保证，DB 层不做唯一约束）。';
COMMENT ON COLUMN departments.sort_order IS
    '同级排序权重，升序排列。默认 0。';

CREATE INDEX IF NOT EXISTS idx_departments_parent_id ON departments(parent_id);

-- 复用 users 表的 updated_at 触发器函数
CREATE TRIGGER set_departments_updated_at
    BEFORE UPDATE ON departments
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- ── users.dept_id ─────────────────────────────────────────────
--
-- 用户所属部门。NULL 表示"无部门"（存量用户一律 NULL）。
-- ON DELETE SET NULL：删部门时用户变成"无部门"，而不是被级联删除。
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS dept_id UUID REFERENCES departments(id) ON DELETE SET NULL;

COMMENT ON COLUMN users.dept_id IS
    '所属部门 ID；NULL 表示无部门。ON DELETE SET NULL：删部门时用户变成无部门。';

CREATE INDEX IF NOT EXISTS idx_users_dept_id ON users(dept_id);

-- ── 种子数据 ──────────────────────────────────────────────────
--
-- 只种一个"总公司"根部门，其余由管理员在界面上建。
-- 不种深层结构：部门是业务数据，不是系统功能，
-- 种一棵假树只会让管理员删掉它（而删除有子部门的节点会被 RESTRICT 挡住）。
INSERT INTO departments (id, parent_id, name, sort_order) VALUES
    ('8f000000-0000-4000-8000-000000000001', NULL, '总公司', 0)
ON CONFLICT (id) DO NOTHING;
