# Changelog

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [0.2.0] - 2026-09-17

主题：**闭环与可部署基线**。不新增业务模块，把「声明了但未生效」的能力补齐或删除，
并建立可回归的验证边界。

### 新增

- 启动时自动执行数据库迁移（`sqlx::migrate!` 内嵌迁移，`MIGRATE_ON_STARTUP` 可关闭），空库可直接启动
- `/api/health` 真实探测数据库与 Redis，依赖异常返回 503 并给出组件状态
- JWT 增加 `jti`：登出只注销当前令牌，不再影响该用户其他设备
- 用户级会话吊销：改密 / 停用 / 删除账号 / 变更角色后，存量令牌立即失效
- 登录爆破防护：账号 + 客户端 IP 双维度失败计数，超阈值返回 429
- `TRUST_PROXY_HEADERS` 开关决定是否采信 `X-Forwarded-For`，默认取 TCP 真实来源
- 审计日志真正接线：受保护请求写入 `audit_logs`（含 `request_id` 关联日志）
- 字典读取接口 `/api/dict/{code}/items`，对任意已登录用户开放
- 集成测试（`tests/api_integration.rs`，覆盖迁移/会话/权限/一致性/文档契约）
- 前端单元测试（Vitest）与 ESLint 9 扁平配置
- `scripts/test_env.sh`：无需 Docker 即可拉起本地集成测试依赖

### 修复

- **登出后重新登录会让已注销令牌复活**：黑名单按用户 ID 存储且登录时清空，改为按 `jti` 存储
- **角色双数据源**：`users.role` 与 `user_roles` 并存导致"改角色不改权限"，现以 `user_roles` 为唯一来源
- **用户列表角色恒为空**：`UserInfo::from` 丢弃角色列表，改为由角色集合推导主角色
- **接口返回 `Admin/User` 而非 `admin/user`**：`Role` 增加 `serde(rename_all = "lowercase")`，与前端类型一致
- **改字典后最长 1 小时读到旧值**：缓存失效此前是空实现，现真正删除对应缓存键
- **删除字典项失败被静默吞掉**：`execute().ok()` 改为返回错误（不存在时 404）
- **批量删除/角色分配非事务**：删除交由外键级联，角色替换在单事务内完成
- **可删除最后一名管理员 / 删除自己**：补充守卫（含批量删除整批判断）
- **500 无任何服务端日志**：`InternalServerError` 现在会记录具体原因
- **`ValidationFailed` 返回 401**：拆分为 `InvalidCredentials`(401) 与 `ValidationFailed`(400)
- **端口不一致**：前端开发代理默认从 9527 修正为与后端一致的 8080
- **Docker HEALTHCHECK 无效**：此前是"再启动一个服务进程"，改为探测 `/api/health`
- **Redis 故障时请求挂起**：为连接管理器设置重试/连接/响应超时，改为快速 503
- **`pnpm lint` 从未可用**：`.eslintrc.cjs` 是 ESLint 8 格式，迁移为 ESLint 9 扁平配置
- **`/api/*` 未知路径返回 401**：鉴权中间件由 `layer` 改为 `route_layer`，未知路径正确返回 404

### 安全（删除无实际作用的模块）

- 删除请求体加密 `CryptoService` 及 `CRYPTO_*` / `RSA_*` 配置：从未接入任何中间件，属死能力
- 删除验证码中间件：只校验答案位数、不与 token 比对，等价于不设防（`generate_captcha` 也无调用方）
- 删除 SQL 注入关键词黑名单中间件：SQLx 已是参数化查询，该中间件会误拦合法内容且覆盖不到 query string
- 口令传输模型统一为「HTTPS 明文 → 服务端 Argon2」；前端移除 SHA-256 预哈希
- "记住密码"不再把口令写入 localStorage，只记忆用户名
- 生产环境拒绝示例占位 `JWT_SECRET` 与通配符 CORS 来源

### 变更（破坏性）

- 迁移 `006` 删除 `users.role` 列（先把数据回填进 `user_roles`）
- 口令格式变更：存量账号在下次登录时自动透明升级（Argon2(sha256) → Argon2(明文)）
- 新增 `jti` 后，v0.1 签发的令牌失效，需重新登录
- 字典读取路径 `GET /api/admin/dict/{code}/items` → `GET /api/dict/{code}/items`
- 移除 `PUT /api/admin/users/{id}/roles`（非事务、吞错且未被前端使用）
- 移除配置：`DATABASE_READ_URL`、`DB_READ_POOL_MAX_SIZE`、`CRYPTO_*`、`RSA_*`、`CAPTCHA_ENABLED`、`APP_SECRET`
- `REDIS_URL` 由"可选"改为必需（此前文档标注可选，实际启动即强依赖）
- `/api/health` 返回体新增 `data.{status,database,redis}` 字段

### 删除的死代码

`CrudTemplate`（拼接 SQL）、读写分离连接池（读库从未被使用）、Redis 高级缓存辅助函数、
分页 `keyword` 相关字段、未使用的 `validator` 依赖等。clippy 警告数 **35 → 0**。

### 已知限制（未在本次范围内）

- OpenAPI 仍为手写规范，尚未由代码生成（已有"文档路由必须真实存在"的集成测试兜底）
- 前端导航仍为静态路由表，未由后端菜单数据驱动
- 按钮级权限为基于角色的显隐，无独立权限码体系
- 接口耗时统计保存在进程内，多副本不聚合、重启丢失
- 审计日志未实现自动保留/清理策略
- 前端 50 个文件未经过 Prettier 统一格式化（未纳入 CI 门禁）
