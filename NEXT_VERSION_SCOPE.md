# Axum Admin 下一版本（v0.2.0）最优改进范围评估

> 评估对象：https://github.com/ZhongGheart/axum-api.git
> 基线提交：`bb0a1b28`（ci: add tests and build pipeline，2026-09-17）
> 评估日期：2026-09-17
> 说明：本文件为评估产出，非上游仓库原有文件。所有路径相对仓库根目录。

---

## 0. 结论摘要

**这个项目的问题不是"功能不够"，而是"广度已经超过闭环能力"。**

仓库已具备企业后台的完整骨架（认证 / RBAC / 菜单 / 字典 / 审计 / 监控 / 导出 / 加密 / 验证码 / 读写分离 / Docker），
但其中**至少 6 项能力只写了代码、没有接线**，另有若干安全语义是错的。同时验证边界很薄：
24 个后端测试全部集中在工具函数与响应模型，没有任何 HTTP / 数据库集成测试，前端 0 测试，
CI 的 clippy 只告警不拦截（实测 35 条 warning）。

因此下一版本的**最优范围不是继续加业务模块，而是一次"闭环版"收敛**：

> **把已声明能力变成真能力，把不可验证变成可回归，让 `docker compose up -d` 在空库上真正可用。**

建议版本号：**v0.2.0「闭环与可部署基线」**，净新增外部功能 = 0。

---

## 1. 评估基线（本次实际执行的验证）

| 项 | 命令 | 结果 |
|---|---|---|
| 后端测试 | `cargo test --locked --all-targets` | ✅ 24 passed / 0 failed |
| 后端静态检查 | `cargo clippy --locked --all-targets` | ⚠️ 35 warnings（未开启 `-D warnings`） |
| 前端类型检查 | `pnpm typecheck` | ✅ 0 error |
| 前端生产构建 | `pnpm build` | ✅ 成功；`vendor-naive` chunk 1153 KB（gzip 300 KB）超 500 KB 警告 |
| 代码规模 | — | 后端 Rust ≈ 6132 行 / 前端 TS+Vue ≈ 6260 行 |
| 提交历史 | — | 46 commits，44 个在 2026-05，2 个在 2026-09 |

**测试分布**：24 个测试全部位于 `src/utils/{jwt,pagination,validation,password,crypto}.rs` 与 `src/model/response.rs`，
即"纯函数 + 序列化"。**没有一个测试覆盖路由、中间件、控制器、Service、Repository 或数据库。**

---

## 2. 现状判定

### 2.1 已声明但运行时未生效的能力（Dead Capability）

这是本版本最该处理的一类问题：**文档/README/提交信息承诺了，但运行时是空转的。**

| # | 能力 | 证据 | 实际后果 |
|---|---|---|---|
| D1 | **RSA/AES 请求体加密** | `CryptoService` 仅在 `src/router/mod.rs:70` 构造并放入 `AppState`，全仓库无任何解密中间件、无公钥下发端点；前端 `frontend/src/utils/crypto.ts` 只做 SHA-256，无 RSA 实现 | `CRYPTO_ENFORCED_PATHS`、`RSA_PRIVATE_KEY`、`RSA_PUBLIC_KEY` 全是死配置。"V9 安全"里这条不成立 |
| D2 | **操作日志（审计）** | `src/middleware/audit_log.rs` 定义了中间件与 `AUDIT_LOG_ENABLED`，但 `src/router/mod.rs` 的 247 行里从未注册它 | `audit_logs` 表永远为空 → "系统日志"页面与审计导出永远无数据；clippy 报 `AUDIT_LOG_ENABLED is never used` |
| D3 | **用户级限流** | `rate_limit_middleware` 作为最外层 `.layer()` 注册（`src/router/mod.rs:233`），而 `AuthenticatedUser` 由更内层的路由级 `auth_middleware` 注入 | 限流执行时 `req.extensions()` 里永远没有用户 → 只保留 IP 限流，`RATE_LIMIT_USER_MAX/WINDOW` 无效果 |
| D4 | **验证码** | `src/middleware/captcha.rs:88-99` 取到 token 后解出 `_challenge/_expected` 却从不比对，只校验"答案 4 位数字"；`generate_captcha()` 无任何调用方与端点 | 任意 `X-Captcha-Token: a-1234` + `X-Captcha-Answer: 1234` 均可通过。开了等于没开，还会因 `CAPTCHA_ENABLED` 默认值不一致（`config` 默认 false、`CaptchaConfig::default` 为 true、`.env.example` 为 true）导致环境行为漂移 |
| D5 | **数据库读写分离** | `DatabasePool::reader()` 全仓库 0 调用（仅 `writer()` 被用），clippy 报 `method reader is never used` | `DATABASE_READ_URL` / `DB_READ_POOL_MAX_SIZE` 是死配置；所有查询仍走主库 |
| D6 | **菜单 / 按钮级权限** | 后端只有 `require_role("admin")`（`src/router/mod.rs` 5 处），无任何 permission code 校验；前端 `MainLayout.vue` 的侧栏用**硬编码数组** `menuOptions`，从不调用 `menuApi.list()` | "菜单管理"页可以增删改菜单，但改完对导航没有任何影响；`menus.permission`、`role_menus` 表、`v-permission` 指令（注册但全仓库 0 处使用）都是摆设 |
| D7 | **参数校验框架** | `Cargo.toml` 声明 `validator` 依赖，但源码中只在 `src/utils/validation.rs:3` 的注释里出现；`validate_phone/validate_password/validate_page` 均被 clippy 判为未使用 | 校验实际靠各 controller/service 里手写的 `if len < 6` 等散落逻辑，规则不一致、不可复用 |

### 2.2 正确性 / 安全缺陷（会真实造成问题）

| # | 问题 | 证据 | 影响 |
|---|---|---|---|
| S1 | **登出会让"已登出的旧 token 复活"** | JWT 无 `jti`（`src/utils/jwt.rs` Claims 仅 sub/role/roles/iat/exp）；黑名单 key 用 `sub`（用户 ID，`src/utils/redis.rs:76`）；`AuthService::login` 登录成功后调用 `remove_token_blacklist(user.id)` | 设备 A 登出 → 设备 B 重新登录 → **设备 A 的 token 恢复有效**。同时"登出"实际是踢掉该用户全部会话，无法单设备登出 |
| S2 | **Redis 挂掉时认证 fail-open** | `src/middleware/auth.rs:88` `is_token_blacklisted(...).unwrap_or(false)` | Redis 异常时黑名单校验被跳过，登出/封禁失效 |
| S3 | **Redis 实为强依赖** | `src/router/mod.rs:59-65` 连接失败仅 `warn!` 然后 `?` 上抛 | README 写 "REDIS_URL 否（可选）"，实际 Redis 不可用则服务**启动失败** |
| S4 | **`ValidationFailed` 映射成 401** | `src/error/mod.rs:62` | 用户名/密码错误返回 401 尚可，但该变体被复用于所有校验失败场景，语义错位；前端 401 分支会提示"未授权，请重新登录" |
| S5 | **"记住密码"明文落盘** | `frontend/src/views/login/index.vue` `saveRemembered()` 直接存 `formData.value.password`；`frontend/src/utils/storage.ts` 的 `encode()` 只是 `btoa`（注释却写"XOR + Base64 加密"） | 任何 XSS 或本地访问都能拿到明文密码 |
| S6 | **GET 响应缓存无失效、无用户维度** | `frontend/src/api/index.ts:119` 缓存所有 GET；key 仅为 `method:url:params`（`frontend/src/utils/cache.ts:96`）；`requestCache.invalidate()` 全仓库 0 调用 | 同一标签页内 A 登出、B 登录后，30s 内仍可能读到 A 的 `/auth/me`、列表等数据；写操作后列表不刷新 |
| S7 | **SQL 注入防护中间件是负收益** | `src/middleware/sql_injection.rs` 对 `/api/auth/*`、`/api/admin` 的 JSON body 做子串黑名单（`"--"`、`" DELETE "`、`"<script"`…） | SQLx 已是参数化查询，真正的注入面为零；该中间件却会**拦截合法内容**（如字典项文案含 "delete"、备注含 `--`），且不覆盖 query string，绕过成本≈0。XSS 也不该在网关层用黑名单解决 |
| S8 | **`CrudTemplate` 用 `format!` 拼表名/字段名** | `src/service/crud.rs:45/71/82/100/119`；`exists(pool, table, field, value)` 的 `field` 直接插值 | 当前调用方都是常量，暂无漏洞；但这是"等一个人传用户输入就炸"的结构性隐患 |
| S9 | **批量删用户 / 改角色非事务 + 吞错误** | `src/controller/user.rs:129-131`（`let _ =` 删 `user_roles`）、`src/controller/user.rs` `assign_user_roles`（`DELETE` 与 `INSERT` 各自 `.ok()`） | 中途失败会留下"用户还在但角色没了"或"角色清空后未写入"的不一致状态；也没有禁止删除自己 / 删掉最后一个 admin |
| S10 | **`UserInfo::from(user)` 丢弃角色列表** | `src/model/user.rs:110` 固定 `roles: Vec::new()` | 用户列表/新建/更新/改状态接口返回的 `roles` 恒为 `[]`，只有 `/api/auth/me` 走 `with_roles` 才有值；前端用户表格因此显示不出真实角色 |
| S11 | **角色双写、双数据源** | `users.role`（`migrations/001`）与 `user_roles` 表（`002`）并存；`src/controller/user.rs` 更新 `users.role`，角色分配写 `user_roles`，登录时 `role` 取前者、`roles` 取后者 | 管理端改 `users.role` 不会改变权限；角色管理页改 `user_roles` 又不会改变 JWT 里的 `role`。两处显示会互相矛盾 |
| S12 | **启动/部署链路断裂** | `src/main.rs` 与 `DatabasePool::new` 均不执行迁移（全仓库无 `sqlx::migrate!`）；`docker-compose.yml` 无迁移步骤/服务 | 空数据卷执行 `docker compose up -d` → `create_router` 调 `init_defaults()` 执行 `SELECT COUNT(*) FROM roles` → 表不存在 → `?` 上抛 → **API 容器启动失败并反复重启**。README 的"一键部署"不成立 |
| S13 | **Docker 健康检查写错** | `Dockerfile` `HEALTHCHECK CMD ["/app/axum-api"]` | 该命令是"再启动一个服务进程"，不是探活；端口已占用必然失败 → 容器长期 unhealthy |
| S14 | **`/api/health` 不做真实探活** | `src/controller/auth.rs` health 返回静态 `ApiResponse::success` | DB/Redis 全挂也返回 200，对编排/负载均衡没有任何指示意义 |
| S15 | **限流可被伪造头绕过** | `src/middleware/rate_limit.rs:36-47` 直接信任 `X-Forwarded-For` / `X-Real-IP` 的第一个值 | 直连 8080（compose 默认暴露）时，客户端自带该头即可任意绕过 IP 限流；登录爆破只靠这一层 |
| S16 | **JWT 密钥未做强度校验** | `src/config/mod.rs` 只要求存在；`docker-compose.yml` 默认值 `your-super-secret-jwt-key-change-in-production` | 默认密钥可上线，签名可被伪造 |
| S17 | **CORS 默认全开** | `src/config/mod.rs` 默认 `*` + `Any` methods/headers | 管理后台默认允许任意源跨域调用 |
| S18 | **开发代理端口不一致** | `frontend/vite.config.ts` proxy `target: http://localhost:9527`，后端默认 `SERVER_PORT=8080`，README 也写 8080 | 按 README 跑起来前后端连不通 |
| S19 | **`dict` 查询接口被限成 admin** | `frontend/src/api/dict.ts` `getCachedDict()` → `/admin/dict/{code}/items`，该路由组带 `require_role("admin")` | `DictSelect` 是通用业务组件，任何非 admin 页面使用它都会 403 |
| S20 | **CSRF/会话语义缺失** | token 存 `localStorage`，无 refresh token，`JWT_EXPIRATION_SECONDS` 默认 7 天 | 长有效期 token + localStorage，XSS 影响面被放大；无轮换、无吊销粒度 |

### 2.3 工程与验证短板

- **测试金字塔缺失**：0 集成测试、0 DB 测试、0 前端测试（`frontend/package.json` 无 test 脚本/依赖）。
- **CI 只做"体检"不做"体检拦截"**：clippy 无 `-D warnings`（35 条 warning 可正常通过）；无 `cargo fmt --check`；`pnpm lint` 未纳入 CI；无 Docker 构建与 compose 冒烟；无依赖安全扫描；CI 不跑迁移。
- **接口契约靠手抄**：`src/docs/mod.rs` 512 行手写 OpenAPI 描述 30+ 路径，与路由/ DTO 无任何编译期或测试期关联，必然漂移。
- **前端产物偏大**：`vendor-naive` 1153 KB（gzip 300 KB）单 chunk，超 Vite 500 KB 告警线；ECharts 无论页面是否需要都被拆成独立 chunk 但仍在首屏预加载链路。
- **可观测性半成品**：`api_metrics` 为进程内 `HashMap`，重启即丢、多副本不聚合；`tracing-subscriber` 虽启用 `json` feature 但 `init_tracing` 仍用默认 fmt 输出；`request_id` 的 span 字段 `record("request_id")` 没有在任何 `#[instrument]` span 中声明，实际不会出现在日志里。
- **一致性成本**：11 处 `#[allow(dead_code)]` 用来压掉"写了没人用"的告警，说明 dead capability 已经常态化。

---

## 3. 下一版本最优范围建议：v0.2.0「闭环与可部署基线」

### 3.1 范围原则

1. **不做新业务模块**。当前 6+ 项能力未闭环，新增模块只会让死代码比例继续上升。
2. **每一项半成品都必须"接线或删除"二选一**，不允许保留第三种状态。
3. **每个修复都要落在可回归的验证边界上**（集成测试 / CI 门禁 / 冒烟脚本）。
4. **README 的能力清单必须与运行时一致**，宁可删功能也不许虚标。

### 3.2 工作流 A — 可启动 / 可部署（P0，约 1.5 人日）

| 项 | 动作 | 修掉 |
|---|---|---|
| A1 | 在 `create_router` 之前执行 `sqlx::migrate!("./migrations").run(&pool)`，并提供 `MIGRATE_ON_STARTUP` 开关；或在 compose 中增加一次性 `migrate` 服务 | S12 |
| A2 | `/api/health` 改为真实探活：`SELECT 1` + Redis `PING`，异常返回 503 | S14 |
| A3 | `Dockerfile` HEALTHCHECK 改为调用上面这个真实探活（`wget -qO- http://127.0.0.1:8080/api/health`，镜像内补 `wget`/`curl`） | S13 |
| A4 | compose 收敛：`JWT_SECRET` 去掉弱默认值（缺失即失败）、postgres/redis 默认不对外发布端口、`web` 依赖 `api` 健康、移除已废弃的 `version:` 字段、前端基础镜像与 pnpm 版本对齐 CI | S16, S12 |
| A5 | `vite.config.ts` 代理目标改为读 `VITE_PROXY_TARGET`，默认 `http://localhost:8080` | S18 |
| A6 | 修 `app` 的启动自检：Redis 不可用时的行为显式化（见 B2），不再"warn 后继续 `?`" | S3 |

**验收**：`docker compose down -v && docker compose up -d` 在空数据卷上 30s 内服务健康，`admin/admin123` 可登录。

### 3.3 工作流 B — 安全与会话闭环（P0/P1，约 4 人日）

| 项 | 动作 | 修掉 |
|---|---|---|
| B1 | JWT 增加 `jti`；黑名单改为 `token:blacklist:{jti}`；**删除登录时的 `remove_token_blacklist`**；另加 `user:revoked_before:{sub}` 用于"踢出全部会话" | S1 |
| B2 | 明确 Redis 故障策略：认证/限流路径 fail-closed（返回 503）或显式降级开关，禁止静默 `unwrap_or(false)`；同时修正 README 的"可选"表述 | S2, S3 |
| B3 | **加密能力二选一**：要么补齐 `GET /api/auth/public-key` + 解密中间件 + 前端 WebCrypto RSA-OAEP 全链路并加测试；要么整体删除 `src/utils/crypto.rs`、`CryptoConfig`、相关环境变量与 `rsa/aes-gcm/sha1` 依赖。**本版本推荐"删除"**（HTTPS 已覆盖传输安全，客户端 SHA-256 反而制造 pass-the-hash 语义） | D1, S-xs |
| B4 | **验证码二选一**：要么实现 Redis 存储的一次性验证码（`GET /api/auth/captcha` + 答案不随 token 下发 + 60s TTL + 失败计数），要么删除该中间件与配置。推荐先删除，等 B5 的真实登录防护做完再评估 | D4 |
| B5 | 登录防护补齐：按"账号 + IP"维度做失败计数与指数退避/锁定；限流不再信任客户端 `X-Forwarded-For`（仅信任受信代理，或用 `ConnectInfo` 取真实 socket IP） | S15, D3 |
| B6 | 密码传输模型定一种并写进文档：推荐"明文经 TLS → 服务端 Argon2"，去掉前端 SHA-256 预哈希；同时移除"记住密码"明文落盘（只记用户名，或改用 refresh token） | S5 |
| B7 | 启动时校验 `JWT_SECRET` 长度/熵，拒绝占位值；生产环境默认关闭 `CORS_ALLOWED_ORIGINS=*` | S16, S17 |

### 3.4 工作流 C — 权限与数据一致性闭环（P1，约 3 人日）

| 项 | 动作 | 修掉 |
|---|---|---|
| C1 | 角色单一数据源：保留 `user_roles` 为准，`users.role` 改为派生的"主角色"（触发器或视图维护），并让 `UserInfo::from` 通过 `with_roles` 统一填充角色 | S10, S11 |
| C2 | 引入 permission code 校验中间件（`require_permission("system:user:delete")`），权限来源 = `role_menus → menus.permission`；**要么真正接线，要么删除 `permission` 字段与 `v-permission` 的宣称** | D6 |
| C3 | 补齐"已登录非 admin"能力层：字典读取、自身资料等改为仅要求认证，不再挂 `require_role("admin")` | S19 |
| C4 | 事务化：`batch_delete_users`、角色分配改为单事务；禁止删除自己、禁止移除最后一个 admin；替换 `CrudTemplate::delete_many` 的裸拼接 | S9 |
| C5 | 错误契约统一：`ValidationFailed → 400/422`；建立稳定业务错误码枚举，不再用 HTTP 码兼任业务码；所有中间件错误体与 `ApiResponse` 对齐 | S4 |
| C6 | `CrudTemplate` 收敛：表名/字段名改为静态白名单或直接下沉到各 Repository 的强类型查询，消除 `format!` 拼 SQL 的入口 | S8 |
| C7 | 审计日志接线：注册 `audit_log_middleware`（放在认证之后以取到用户），写入前脱敏 password/token，增加保留策略与清理任务；修掉 `request_id` 的 span 字段 | D2, observability |

### 3.5 工作流 D — 可验证边界（P1，约 3 人日）

| 项 | 动作 |
|---|---|
| D1 | 后端集成测试（`testcontainers` 起 Postgres+Redis，或用 compose 起依赖）：覆盖①空库迁移成功 ②登录→`/auth/me` ③无权限 403 ④登出后旧 token 401 ⑤管理员用户 CRUD 完整链路 |
| D2 | 前端引入 `vitest` + `@vue/test-utils`：覆盖请求拦截器（token 注入 / 401 处理 / 缓存 key 含用户维度）、`useUserStore`、路由守卫 |
| D3 | CI 门禁升级：`cargo fmt --check`、`cargo clippy -- -D warnings`、`pnpm lint`（去掉 `--fix`）、`docker build` 两个镜像、`docker compose up` 冒烟、`cargo audit` / `pnpm audit` |
| D4 | 契约单一来源：用 `utoipa`（或 schemars + builder）替换 512 行手写 OpenAPI，并加"每个注册路由都出现在 spec 中"的测试；前端 TS 类型改为从 spec 生成 |
| D5 | 前端数据一致性：GET 缓存 key 加入用户/租户维度、写操作后 `invalidate`、敏感接口（`/auth/me`）禁缓存 |

### 3.6 明确不做（v0.2.0 Out of Scope）

- ❌ 新业务模块（文件中心、消息通知、工作流、多租户、报表）
- ❌ 微服务化 / K8s / 服务网格
- ❌ **读写分离**：当前是死配置，v0.2 直接删除（`DATABASE_READ_URL`、`DB_READ_POOL_MAX_SIZE`、`DatabasePool.reader`），等真有从库再引入
- ❌ CDN 外部化 / 打包体积深度优化（保留现状，仅记录 chunk 警告）
- ❌ 监控体系扩展（Prometheus 指标导出、告警通道）——留到 v0.3
- ❌ UI 改版与新页面

---

## 4. 迭代切分与工作量

| 迭代 | 内容 | 产出 | 预估 |
|---|---|---|---|
| **PR-1 可部署** | A1–A6 | 空库 `docker compose up -d` 可用；健康检查真实 | 1.5 人日 |
| **PR-2 安全收敛** | B1–B7（含删除 crypto/captcha/sql-injection 与依赖） | 会话语义正确；负收益中间件清零；死配置清零 | 4 人日 |
| **PR-3 权限与数据** | C1–C7 | 单一角色来源；permission 真正生效；事务一致；审计可用 | 3 人日 |
| **PR-4 验证门禁** | D1–D5 | 集成测试 + CI 强门禁 + 生成式 OpenAPI | 3 人日 |
| 收尾 | README 重写（能力清单与运行时对齐）、CHANGELOG、版本号 `0.2.0` | — | 0.5 人日 |

**合计 ≈ 12 人日（单人 2.5 周 / 双人 1.5 周）**，净新增功能 0，删除 > 新增。

---

## 5. 验收标准（Definition of Done）

v0.2.0 可以发布，当且仅当：

1. `docker compose down -v && docker compose up -d` 在空库上一次成功，容器健康，默认账号可登录。
2. 仓库中**不存在"声明了却未接线"的能力**——`grep` 不到 `allow(dead_code)` 掩盖的未使用项，clippy `-D warnings` 通过。
3. `DATABASE_READ_URL` / `CRYPTO_*` / `CAPTCHA_*` 等配置**要么生效、要么从 `.env.example` 与 README 中消失**。
4. 关键链路有集成测试：登录、鉴权、权限不足、登出失效、管理员 CRUD、空库迁移。
5. CI 在 fmt / clippy / lint / test / build / compose 冒烟任一失败时**必须红**。
6. README 的 API 表与能力清单与代码一致；OpenAPI 由代码生成且路由覆盖率 100%。
7. 前端无明文密码落盘；GET 缓存按用户隔离且在写操作后失效。

---

## 6. 备选方案（若目标只是"演示/作品集"而非生产可用）

若下一版本的定位是**对外展示**而非上线，则最优范围会变成另一组取舍（约 3 人日）：
优先修 A4/A5（一键起得来）、A3（健康检查）、C1（角色显示）、D2（审计接线），
再补一个可截图的新页面；可以接受 crypto/captcha/读写分离继续保留为"预留"。

**但不建议**：这会让"看起来有 12 个能力、实际 6 个空转"的状态固化，
后续每加一个模块，维护成本按当前 dead code 比例线性上升。

---

## 附录 A：核心证据索引

| 主题 | 位置 |
|---|---|
| 路由与中间件装配 | `src/router/mod.rs:53-247` |
| JWT Claims（无 jti） | `src/utils/jwt.rs:11-25` |
| 黑名单按用户 ID | `src/utils/redis.rs:68-108` |
| 登录清除黑名单 | `src/service/auth.rs` `login()` |
| 鉴权 fail-open | `src/middleware/auth.rs:85-92` |
| 验证码未校验答案 | `src/middleware/captcha.rs:63-100` |
| SQL 黑名单中间件 | `src/middleware/sql_injection.rs:22-36, 100-130` |
| 用户级限流失效 | `src/middleware/rate_limit.rs:63-92`（与 `src/router/mod.rs:233` 的层序） |
| 审计中间件未注册 | `src/middleware/audit_log.rs:36`（无引用） |
| 读库从未使用 | `src/repository/db.rs:63-70`（`reader()` 无调用） |
| 通用 CRUD 拼 SQL | `src/service/crud.rs:45,71,82,100,119` |
| 批量删/改角色无事务 | `src/controller/user.rs:122-131` 及 `assign_user_roles` |
| 角色双数据源 | `migrations/001_create_users.sql`（`users.role`）与 `migrations/002_create_rbac.sql`（`user_roles`） |
| `UserInfo::from` 丢角色 | `src/model/user.rs:104-118` |
| 无自动迁移 | `src/repository/db.rs`、`src/main.rs`（无 `sqlx::migrate!`） |
| Docker 健康检查 | `Dockerfile` HEALTHCHECK |
| compose 缺迁移 | `docker-compose.yml` |
| 前端 GET 缓存 | `frontend/src/api/index.ts:55-70,115-120`、`frontend/src/utils/cache.ts:96-101` |
| 记住密码明文 | `frontend/src/views/login/index.vue` `saveRemembered()`、`frontend/src/utils/storage.ts` `encode()` |
| 硬编码导航菜单 | `frontend/src/layouts/MainLayout.vue:121-234` |
| 手写 OpenAPI | `src/docs/mod.rs`（512 行） |
| 开发代理端口 | `frontend/vite.config.ts` `server.proxy['/api'].target = 9527` |

## 附录 B：执行清单（可直接转 Issue）

**P0**
- [ ] A1 启动执行迁移 + compose 迁移步骤
- [ ] A2 `/api/health` 真实探活（DB+Redis）
- [ ] A3 修 Dockerfile HEALTHCHECK
- [ ] A4 compose 安全默认值收敛
- [ ] B1 JWT `jti` + 按 token 黑名单 + 移除登录清黑名单
- [ ] B2 Redis 故障策略显式化
- [ ] B3 删除或补齐请求加密（推荐删除）
- [ ] B5 登录失败锁定 + 可信客户端 IP
- [ ] C1 角色单一数据源 + `UserInfo` 角色填充

**P1**
- [ ] B4 删除或补齐验证码
- [ ] B6 统一密码传输模型 + 移除明文记住密码
- [ ] B7 启动校验 JWT_SECRET / 收紧 CORS 默认值
- [ ] C2 permission code 校验或删除宣称
- [ ] C3 字典等接口降级为"仅认证"
- [ ] C4 事务化 + 防自删/最后 admin
- [ ] C5 错误码契约统一
- [ ] C6 消除 `format!` 拼 SQL
- [ ] C7 审计中间件接线 + 脱敏 + 保留策略
- [ ] D1 后端集成测试
- [ ] D2 前端单测
- [ ] D3 CI 强门禁
- [ ] D4 OpenAPI 代码生成 + 覆盖率测试
- [ ] D5 前端缓存按用户隔离与失效
- [ ] 删除读写分离死配置（A 之外单独清理）
- [ ] README 能力清单与运行时对齐
