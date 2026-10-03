-- v0.19.0：用户名 / 邮箱大小写归一（trim + 小写）
--
-- 用户名不只是展示用：它是**登录键**，也是管理员在用户列表里辨认账号的依据。
-- 而 Postgres 的 `UNIQUE(username)` 是**大小写敏感**的，于是
-- `Admin` / `ADMIN` / `aDmIn` 能与真 `admin` 并存——自助注册一次就能造出来，
-- 管理员在列表上看到 `Admin` 无从判断它是不是真 admin。
-- 钓鱼、社工、"给 admin 绑个角色"这类操作都会被引到伪造账号上。
-- 邮箱同理：`Case@Test.com` 与 `casetest@com` 会登录到两个不同的 id。
--
-- ── 为什么这里**不能**照抄 008（角色名）的做法 ──────────────────────
-- 008 对冲突行是**跳过**的，理由写在它的注释里：
-- 「迁移期不该把整个应用卡在起不来」。角色名这样可以，用户名不行：
--
--   跳过之后，那一行会变成**登录不到的孤儿账号**。
--   因为本迁移之后所有写入都是小写，`find_by_username("admin")`
--   只命中小写那一行；旧的 `Admin` 行再也登不进去，
--   却仍然挂在库里、仍然持着原来的角色，且在用户列表里与真 admin
--   **肉眼无法区分**。
--
--   那等于"声称修好了大小写唯一性"，实际反而留下一个更难查的冒充入口——
--   比迁移前的状态更糟。所以这里选择**报错让应用起不来**，
--   并把冲突的账号列出来，让管理员显式决定合并还是改名。
--   应用起不来是可见的故障；一个静默的冒充入口不是。
--
-- ── 归一用 regexp_replace 裁两端空白，不写 btrim(name, '[:space:]') ──
-- 理由逐字见 008：btrim 的第二参数是**字符集合**而非字符类，
-- 会逐字符匹配 `'[' ':' 's' 'p' 'a' 'c' 'e' ']'`，
-- 实测把 admin 裁成 dmin、把 auditor 裁成 uditor。

-- 1) 归一 username：先处理无冲突的那些
UPDATE users u
SET username = lower(regexp_replace(u.username, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
WHERE regexp_replace(u.username, '^[[:space:]]+|[[:space:]]+$', '', 'g') <> ''
  AND u.username <> lower(regexp_replace(u.username, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
  AND NOT EXISTS (
      SELECT 1 FROM users other
      WHERE other.id <> u.id
        AND lower(regexp_replace(other.username, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
          = lower(regexp_replace(u.username, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
  );

-- 2) 归一 email：同上
UPDATE users u
SET email = lower(regexp_replace(u.email, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
WHERE regexp_replace(u.email, '^[[:space:]]+|[[:space:]]+$', '', 'g') <> ''
  AND u.email <> lower(regexp_replace(u.email, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
  AND NOT EXISTS (
      SELECT 1 FROM users other
      WHERE other.id <> u.id
        AND lower(regexp_replace(other.email, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
          = lower(regexp_replace(u.email, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
  );

-- 3) 冲突报出：归一之后仍有重名，就**拒绝启动**而不是静默合并
DO $$
DECLARE
    dup_username text;
    dup_email    text;
BEGIN
    SELECT string_agg(grouped, ' | ') INTO dup_username
    FROM (
        SELECT lower(username) || ' → ' || string_agg(username, ', ') AS grouped
        FROM users
        GROUP BY lower(username)
        HAVING count(*) > 1
        ORDER BY lower(username)
    ) s;

    SELECT string_agg(grouped, ' | ') INTO dup_email
    FROM (
        SELECT lower(email) || ' → ' || string_agg(email, ', ') AS grouped
        FROM users
        GROUP BY lower(email)
        HAVING count(*) > 1
        ORDER BY lower(email)
    ) s;

    IF dup_username IS NOT NULL OR dup_email IS NOT NULL THEN
        RAISE EXCEPTION E'
用户名/邮箱大小写冲突：归一后出现重名，迁移拒绝静默合并。
合并账号会丢权限（user_roles 按 user_id 关联，任选一行留下就等于
把另一个账号的角色丢掉），所以这必须由管理员显式决定。

  用户名冲突（归一后）: %
  邮箱冲突（归一后）  : %

处理办法：改名或删除多余的那一行，然后重新启动。',
            coalesce(dup_username, '（无）'),
            coalesce(dup_email, '（无）');
    END IF;
END $$;

-- 4) 函数唯一索引：把"大小写不敏感唯一"落到数据库层
--
-- 有了应用侧归一，`UNIQUE(username)` 事实上已经够了（存下去的都是小写）。
-- 这两个索引是**第二道**：任何绕过应用直接写库的路径
-- （手工 SQL、导数据、将来某个漏了归一的新写入口）都会被这里挡住，
-- 而不是等到"用户列表里出现两个肉眼一样的账号"才被发现。
CREATE UNIQUE INDEX IF NOT EXISTS users_username_lower_key ON users (lower(username));
CREATE UNIQUE INDEX IF NOT EXISTS users_email_lower_key ON users (lower(email));
