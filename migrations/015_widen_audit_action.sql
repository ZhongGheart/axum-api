-- 拓宽 `audit_logs.action`
--
-- `action` 由中间件拼成 `"{method} {path}"`，而 `path` 本身是 VARCHAR(500)——
-- 两列装的是同一段信息，宽度却差 5 倍，于是**只要路径再长一点，审计就整条静默消失**。
--
-- 实测（v0.20.0）：`POST /api/admin/users/{id}/sessions/{jti}/revoke`
-- 拼出来是 107 字符，超出 VARCHAR(100)，INSERT 直接失败；
-- 而中间件是 `tokio::spawn` 里写的，失败只留一行 `tracing::warn!`，
-- 请求照常返回 200 —— 表现是"这个操作没有审计记录"，而不是"审计写不进去"。
--
-- 这不是 v0.20.0 引入的缺陷，而是新端点第一次把路径推过了 100 字符这条线。
-- 改列宽而不是截断 action：action 的唯一用途就是检索，
-- 一条被截掉尾部的 action 检索不到，等于没有。
ALTER TABLE audit_logs
    ALTER COLUMN action TYPE VARCHAR(512);

COMMENT ON COLUMN audit_logs.action IS
    '"{method} {path}"，例如 "POST /api/admin/users/{id}/sessions/{jti}/revoke"。宽度必须容得下最长的 path（VARCHAR(500)）加上方法名，否则超长路径的审计会被整条丢弃。';
