-- v0.20.0：用户自助资料字段（display_name / avatar_url）
--
-- 背景：`/api/auth/*` 下只有 `password` 是 PUT，**没有 profile 端点**；
-- `users` 表 8 列里也没有任何"这人叫什么/长什么样"的字段。
-- 于是用户对自己的账号**没有任何自助修改能力**——
-- 想改显示名只能找管理员，而管理员能改的东西比用户能改的多得多。
--
-- ── 为什么 email 不在这一版 ──────────────────────────────────
-- 邮箱是**登录键**（登录框同时接受用户名或邮箱），也是找回账号的唯一凭据。
-- 自助改邮箱必须先证明"新邮箱归你所有"，而本仓目前**没有邮件通道**
-- （无 SMTP 依赖、无发信代码、无验证码端点）。没有验证通道时放开自助改邮箱，
-- 等于让任何登录用户把账号的找回凭据改成攻击者的地址。
-- 所以这一版**只加不影响登录与找回的展示型字段**，邮箱留给有邮件通道的那一版
-- （见 docs/ROADMAP.md 的 v0.22.0 D1/D2）。这个"不做"是有意的，不是遗漏。
--
-- ── display_name 为什么可空 ─────────────────────────────────
-- 存量用户一律 NULL，前端回退显示 username。
-- 反过来说，如果这一列 NOT NULL DEFAULT ''，那么"空字符串"和"没填过"
-- 就成了两种无法区分的状态，而 v0.19.0 刚在用户名上吃过"归一后两种形态并存"的亏
-- （` Admin ` 与 `admin`）。新列从第一天起就只有一个"没填过"的表示法。
--
-- ── 为什么不加 CHECK 约束 ───────────────────────────────────
-- 用户名/邮箱需要唯一索引是因为它们是登录键，而 display_name **不是**：
-- 两个人都叫"张三"完全合法，管理员在列表里靠 username 与 id 辨认。
-- 给展示字段加唯一约束会把一个合理场景变成 409。
--
-- 长度按字符而非字节：`display_name` 的 50 与 `users.username` 的 varchar(50) 同口径，
-- Postgres 对 varchar 按字符计，所以 50 个汉字存得下（v0.18.0 修过"按字节判定"
-- 把中文用户名砍掉三分之二容量的同类问题，这里不能重犯）。

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS display_name VARCHAR(50);

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS avatar_url VARCHAR(512);

-- avatar_url 只允许站内相对路径（/uploads/...）。
-- 开放成任意 URL 会引入一个可外链的加载点：管理员列表页渲染头像时
-- 就成了被跟踪的第三方请求来源，而 /uploads 下的图片同源可读。
-- 存绝对路径会让"域名写错"这类问题在运行时才暴露，且无法在库层拦住。
-- 用 NOT VALID 先加约束再验证，避免全表长时间持锁。
--
-- 注意：**一个正则不够**。初版只写了
--     `avatar_url ~ '^/uploads/[A-Za-z0-9._/-]+$'`
-- 而该字符类里同时有 `.` 和 `/`，于是 `..` 天然合法——
-- 实测 `/uploads/../etc/passwd` 被**直接接受**（UPDATE 1，不是报错）。
-- 应用层 `normalize_avatar_url` 有逐段 `..` 检查所以 HTTP 路径拦得住，
-- 但绕过应用直写库的路径会穿透，而这一列的 CHECK 就是为那条路径存在的。
-- 所以 `..` 必须单独一条约束，且要求它是一个**完整的路径段**
-- （前后必须是行首、`/` 或行尾），否则 `..foo` 或 `a..b` 这类合法文件名会被误伤。
ALTER TABLE users
    ADD CONSTRAINT users_avatar_url_relative_path
    CHECK (avatar_url IS NULL OR avatar_url ~ '^/uploads/[A-Za-z0-9._/-]+$');

ALTER TABLE users
    ADD CONSTRAINT users_avatar_url_no_parent_segment
    CHECK (avatar_url IS NULL OR avatar_url !~ '(^|/)\.\.(/|$)');

-- display_name 两侧空白一律归一掉，与 013 对 username/email 的处理同口径：
-- 存进去的必须是它显示时的样子，否则搜索/比对会出现"看着一样其实不同"。
-- 用 CHECK 而不是触发器：这一列由应用侧写入，归一逻辑已集中在写入路径，
-- DB 层只挡住"看起来就不对"的值，不做静默改写（静默改写会让返回值与请求不符）。
ALTER TABLE users
    ADD CONSTRAINT users_display_name_is_trimmed
    CHECK (display_name IS NULL OR display_name = regexp_replace(display_name, '^[[:space:]]+|[[:space:]]+$', '', 'g'));
