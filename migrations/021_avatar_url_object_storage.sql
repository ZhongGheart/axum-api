-- v0.27.0（D2 对象存储抽象）：放宽 avatar_url 以容纳对象存储 / CDN 的绝对 URL
--
-- 背景：v0.20.0 把头像写死在本地磁盘，于是 `avatar_url` 被三条约束同时绑在
-- `/uploads/` 这个站内相对路径上：DB 的 CHECK、应用层的 `normalize_avatar_url`、
-- 以及 `ServeDir` 挂载点。v0.27.0 引入存储抽象后，S3 后端返回的是
-- `https://cdn.example.com/avatars/xxx.png`，前两条会把它拒掉——
-- 表现为"对象存进去了，但用户资料写不进去"，且报错是 500 级 CHECK 冲突。
--
-- ── 这次只改 DB，应用层**刻意不放宽** ──
--
-- `normalize_avatar_url` 仍然只接受 `/uploads/` 开头的相对路径。
-- 因为它只挂在 `UpdateProfileRequest` 的反序列化器上，也就是**用户手填的值**；
-- 而上传端点走 service→repo 直连，本来就不经过它。
-- 换句话说：服务端自己生成的 S3 地址要能存，用户手填的任意外链不要能存。
-- 后者一旦放开，任何用户都能让管理员的浏览器在用户列表页
-- 加载攻击者指定的地址（Referer 泄露、追踪像素），而这不是头像功能的需求。
--
-- ── 为什么不能只用一条正则 ──
--
-- 014 的注释已记过一次：字符类里同时有 `.` 和 `/` 时，`..` 天然合法，
-- 实测 `/uploads/../etc/passwd` 被直接接受。所以 `..` 必须单独一条约束，
-- 且要求它是**完整的路径段**（前后是行首、`/` 或行尾），
-- 否则 `..foo`、`a..b` 这类合法文件名会被误伤。
--
-- 绝对 URL 那条同理：`..` 检查必须对它也生效，
-- 所以这里**保留 014 的 `users_avatar_url_no_parent_segment` 不动**，
-- 只替换"形态"那条约束。它本来就写在 `avatar_url` 上，与形态无关。

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_avatar_url_relative_path;

-- 新形态：站内相对路径 **或** http(s) 绝对 URL
--
-- 分成两个正则用 OR 连起来，而不是写成一个大字符类：
-- 后者要同时容纳 `/uploads/` 前缀与 `://`，可读性归零，
-- 而且一旦要加"只允许 http/https"的判断就没地方下手了。
--
-- 绝对 URL 的字符集刻意**不含空格**（POSIX 字符类 `[:space:]` 的排除
-- 需要写成否定形式，这里直接不列入白名单）：`avatar_url` 最终会被塞进
-- `<img src>`，带空格的地址会让"看起来是 URL 的东西"绕过这一层。
-- 用 NOT VALID 先加约束再验证，避免全表长时间持锁。
ALTER TABLE users
    ADD CONSTRAINT users_avatar_url_relative_path
    CHECK (
        avatar_url IS NULL
        OR avatar_url ~ '^/uploads/[A-Za-z0-9._/-]+$'
        OR avatar_url ~ '^https?://[A-Za-z0-9._~:/?#@!$&()*+,;=%-]+$'
    ) NOT VALID;

-- 存量行必然满足新约束（它们的形态没变），但仍显式验证一次，
-- 让"这一步真的跑过了"写在迁移里，而不是靠推断。
ALTER TABLE users VALIDATE CONSTRAINT users_avatar_url_relative_path;

-- 长度上界。014 建列时用的是 VARCHAR(512)，这里再加一条 CHECK
-- 是因为"绝对 URL"比站内路径长得多，而列宽是静默截断的来源：
-- 一个 600 字符的 CDN 地址被截成 512，前端拿到的是半条 URL，
-- 表现为"偶尔某几张头像裂图"，排查成本远高于直接拒掉。
ALTER TABLE users
    ADD CONSTRAINT users_avatar_url_length
    CHECK (avatar_url IS NULL OR char_length(avatar_url) <= 512) NOT VALID;

ALTER TABLE users VALIDATE CONSTRAINT users_avatar_url_length;
