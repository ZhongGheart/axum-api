# Axum Admin — 全栈管理系统

[![CI](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml/badge.svg)](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml)

基于 **Rust Axum** 后端 + **Vue 3** 前端的企业级全栈管理平台。

> v0.9.0 补齐了授权边界的最后一面，并把授权探针从一次性脚本变成仓库里的工具：
> `POST /roles` 此前只查"授予什么"，不查"授予给谁"——只持 `system:user:update`
> 的角色能给纯 admin 账号追加角色（实测 200），而 `update_user` / `delete_user`
> 三处都查目标；追加后也不撤销存量会话，新权限要等目标用户重新登录才生效。
> 现两处都与 `update_user` 对齐，且不阻断"给自己追加弱角色"这条合法的自我降级路径。
> 令牌新增毫秒级 `iat_ms`：JWT 标准的 `iat` 只有秒级精度，分不清"吊销前签发"与
> "吊销后签发"，任何秒级方案只能在**漏吊销**与**误伤新登录**之间二选一，两个都错。
> 探针工具第一次运行就抓到第四个洞：只持 `system:role:delete` 的操作员能删掉
> 承载 `system:log:list` 的角色——与 v0.8.0 的 `delete_menu` 同一形状，
> 授权的两面只装了一面。
> v0.8.0 给菜单**删除**补上了与 v0.7.0 清空同款的授权下界：
> 此前删掉一个承载权限码的按钮，等于把那个码从所有依赖它的角色身上剥掉，
> 而删除路径上没有任何检查——持 `system:menu:delete` 的角色因此可以
> 绕过 `system:menu:grant` 对别的角色撤权。守卫覆盖整棵子树（删除是级联的），
> 且只对"已被授予至少一个角色"的码设限，以免"整理菜单结构"这类无害操作被误伤。
> v0.7.0 给"清空权限码"补上了恢复路径：此前把按钮的权限码清空后，
> 全系统就没有任何角色再持有它，而"改写权限码必须持有目标码"的守卫
> 会把**写回去**也一并拦死。现由清空者本人一键还原。
> v0.6.0 用户支持多角色：此前用户表单只提交单个角色，而后端是整体替换，
> 于是给一个已有角色的人再添一个角色、保存后其余角色会**静默消失**。
> 现在写入口接受完整角色集合，界面如实呈现多标签。
> v0.5.0 补上了角色与授权闭环：授权写路径不再静默失效，
> 并把此前只能改数据库的授权配置接到了界面上；角色也从写死的常量变成了数据——
> 新建一个角色即可在用户表单里分配，无需改代码。变更明细见 [CHANGELOG.md](CHANGELOG.md)。

## 能力清单（与运行时一致）

| 能力 | 状态 | 说明 |
|------|------|------|
| JWT 认证 | ✅ | HS256；令牌带 `jti`，支持**单令牌注销**与**用户级会话吊销** |
| RBAC | ✅ | 角色唯一数据源为 `user_roles` 表；admin 路由再按**权限码**逐接口鉴权 |
| 登录防护 | ✅ | 账号 + 客户端 IP 双维度失败计数，超阈值锁定；客户端 IP 默认取 TCP 真实来源 |
| 限流 | ✅ | 基于 Redis 的固定窗口限流（IP 维度） |
| 操作日志 | ✅ | 受保护路由写入 `audit_logs`（仅方法/路径/查询串，不记录请求体） |
| 数据字典 | ✅ | Redis 缓存 + 写操作真实失效；读取接口对**任意已登录用户**开放 |
| 菜单管理 | ✅ | 菜单是导航的唯一来源：`/api/auth/menus` 决定侧栏与前端动态路由 |
| 权限码 | ✅ | 28 个 `<模块>:<资源>:<动作>` 权限码存于 `menus.permission`（按钮型菜单），后端强制鉴权 + 前端按码判定；清空后可由清空者本人恢复 |
| 角色与授权 | ✅ | 角色增删改 + 菜单/权限码授权树；授权**要么完整成功、要么整体回滚**；内置角色不可删除、不可改名；自定义角色可直接分配给用户 |
| 用户管理 | ✅ | 增删改查、批量删除、状态切换、重置密码；**多角色分配**；含"最后一个管理员"保护 |
| 系统监控 | ✅ | CPU/内存/磁盘、DB/Redis 状态、接口耗时统计（进程内，重启丢失） |
| Excel 导出 | ✅ | 用户列表、操作日志、系统信息 |
| OpenAPI 文档 | ✅ | utoipa 从 handler 注解与 DTO 派生生成；双向覆盖测试保证文档与路由同步 |
| 接口级权限码 | ⚠️ | 仅 admin 路由细化到权限码；非 admin 角色仍被 `require_role("admin")` 整体挡住 |

> 表中 ⚠️ 项是明确的能力边界，不再作为"已实现"宣传。

## 项目结构

```
axum-api/
├── src/                     # Rust 后端源码
│   ├── main.rs              # 二进制入口（日志、优雅关闭）
│   ├── lib.rs               # 库入口（供集成测试直接构建路由）
│   ├── config/              # 配置层（DB、Redis、JWT、CORS、限流、安全策略）
│   ├── router/              # 路由注册 + 全局中间件链
│   ├── controller/          # 控制器层
│   ├── service/             # 服务层（认证、RBAC、监控）
│   ├── repository/          # 数据访问层（SQLx 纯 SQL）
│   ├── model/               # 实体 + DTO 模型
│   ├── middleware/          # JWT 鉴权、限流、请求 ID、审计日志、客户端 IP
│   ├── error/               # 全局异常处理
│   └── utils/               # 工具（JWT、密码、Redis、分页、校验、导出）
├── tests/api_integration.rs # 集成测试（真实 Postgres + Redis）
├── e2e/                     # 真实 Chrome 回归 + 授权探针（零依赖，直连 CDP）
├── frontend/                # Vue 3 前端源码
├── migrations/              # 数据库迁移（启动时自动应用）
├── scripts/test_env.sh      # 本地集成测试依赖（无需 Docker）
├── Dockerfile               # 后端多阶段构建
└── docker-compose.yml       # 全栈编排（pg + redis + api + web）
```

## 技术栈

| 层级 | 技术 | 版本 |
|------|------|------|
| **后端** | Rust + Axum + Tokio | 1.93+ / 0.8 |
| **数据库** | PostgreSQL + SQLx | 16 / 0.8 |
| **缓存** | Redis | 7 |
| **前端** | Vue 3 + TypeScript + Vite | 3.5 / 6 |
| **UI** | Naive UI | 2.41 |
| **接口文档** | utoipa | 5.5 |
| **测试** | cargo test + Vitest | — |
| **部署** | Docker + Docker Compose + Nginx | — |

## 快速开始

### 环境要求

- Rust **1.93+**（`Cargo.lock` 中部分依赖使用 edition2024，1.82 无法构建；Dockerfile 固定 1.93）
- Node.js 18+（CI 使用 22）
- pnpm 10+
- PostgreSQL 16、Redis 7（或使用 Docker Compose）

### 1. 配置环境变量

```bash
cp .env.example .env

# JWT_SECRET 必填且至少 32 字符（生产环境会拒绝示例占位值）
openssl rand -base64 48
# 把输出填入 .env 的 JWT_SECRET
```

### 2. 启动后端

数据库迁移会在启动时自动执行（`MIGRATE_ON_STARTUP=true`），**空库可直接启动**：

```bash
cargo run
# 或 RUST_LOG=debug cargo run
```

> Redis 是**必需**依赖：限流、令牌注销、登录失败计数都依赖它；不可用时服务启动失败，
> 运行中不可用时认证/限流路径会返回 503（fail-closed）。

### 3. 启动前端

```bash
cd frontend
pnpm install
pnpm dev
# 默认监听 http://localhost:3000，按 VITE_API_PROXY_TARGET（默认 http://localhost:8080）代理 /api
```

### 4. 访问

打开 `http://localhost:3000`，使用 `admin / admin123` 登录（首次启动自动创建）。

### Docker Compose 一键部署

```bash
cp .env.example .env
# 必填：DB_PASSWORD、JWT_SECRET（缺失时 compose 会直接报错，不提供弱默认值）
docker compose up -d

docker compose logs -f api
docker compose down          # 停止
docker compose down -v       # 停止并删除数据卷
```

访问 `http://localhost`，Nginx 自动代理 `/api` 到后端。
postgres / redis 默认**不向宿主机暴露端口**，仅在同网络内可达。

## 环境变量

| 变量 | 必需 | 默认值 | 说明 |
|------|------|--------|------|
| `APP_ENV` | 否 | `development` | `production` 下拒绝占位 JWT 密钥与通配符 CORS |
| `SERVER_HOST` | 否 | `0.0.0.0` | 监听地址 |
| `SERVER_PORT` | 否 | `8080` | 监听端口 |
| `DATABASE_URL` | **是** | — | PostgreSQL 连接字符串 |
| `DB_PASSWORD` | Compose **是** | — | docker compose 使用的数据库密码 |
| `MIGRATE_ON_STARTUP` | 否 | `true` | 启动时自动执行迁移 |
| `REDIS_URL` | **是** | `redis://127.0.0.1:6379` | Redis 连接字符串 |
| `JWT_SECRET` | **是** | — | 至少 32 字符，生产环境禁止示例值 |
| `JWT_EXPIRATION_SECONDS` | 否 | `604800` | JWT 过期时间（秒） |
| `CORS_ALLOWED_ORIGINS` | 否 | `http://localhost:3000` | 逗号分隔；生产环境禁止 `*` |
| `DB_POOL_MAX_SIZE` | 否 | `20` | 数据库连接池大小 |
| `DB_CONNECT_TIMEOUT` | 否 | `10` | 连接超时（秒） |
| `RATE_LIMIT_IP_MAX` / `RATE_LIMIT_IP_WINDOW` | 否 | `100` / `60` | IP 限流阈值与窗口 |
| `TRUST_PROXY_HEADERS` | 否 | `false` | 是否信任 `X-Forwarded-For`（仅置于可信代理后时开启） |
| `LOGIN_MAX_FAILURES` | 否 | `10` | 登录失败锁定阈值 |
| `LOGIN_FAILURE_WINDOW` | 否 | `300` | 登录失败计数窗口（秒） |

前端（`frontend/.env.*`）：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `VITE_API_BASE_URL` | `/api` | API 基础路径 |
| `VITE_API_PROXY_TARGET` | `http://localhost:8080` | 开发代理目标（需与后端端口一致） |

## API

统一响应格式：`{ "code": <HTTP 状态码>, "message": string, "data": T | null }`。

### 公开

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/health` | 健康检查（真实探测 DB + Redis，异常返回 503） |
| POST | `/api/auth/register` | 用户注册 |
| POST | `/api/auth/login` | 登录（返回 JWT） |

### 需登录

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/auth/me` | 当前用户信息（含角色列表） |
| GET | `/api/auth/menus` | 当前用户可见的导航菜单树（前端动态路由与侧栏的数据源） |
| POST | `/api/auth/logout` | 注销**当前令牌** |
| GET | `/api/dict/{code}/items` | 读取字典项（任意已登录用户） |

### 仅 admin

| Method | Path | 说明 |
|--------|------|------|
| GET/POST | `/api/admin/users` | 用户列表 / 新建用户 |
| PUT/DELETE | `/api/admin/users/{id}` | 更新 / 删除用户 |
| POST | `/api/admin/users/batch-delete` | 批量删除（含最后管理员保护） |
| PUT | `/api/admin/users/{id}/status` | 启用 / 停用（停用即吊销会话） |
| POST | `/api/admin/users/{id}/reset-password` | 重置密码（并吊销会话） |
| GET/POST | `/api/admin/users/{id}/roles` | 查询 / 追加用户角色 |
| GET/POST | `/api/admin/roles`、`PUT/DELETE /api/admin/roles/{id}` | 角色管理（内置角色不可删除/改名；仍被用户占用的角色拒绝删除；角色名自动归一化，撞名 409） |
| GET/POST | `/api/admin/menus`、`PUT/DELETE /api/admin/menus/{id}` | 菜单管理（改动即时影响前端导航） |
| PUT | `/api/admin/roles/{id}/menus` | 角色-菜单关联（**全量覆盖**；任一 ID 非法即整体 400） |
| GET | `/api/admin/menus?role_id={id}` | 某角色已授权的菜单树（授权弹窗的默认勾选值） |
| GET/POST | `/api/admin/dict/types`、`PUT/DELETE /api/admin/dict/types/{id}` | 字典类型管理 |
| GET/POST | `/api/admin/dict/items`、`PUT/DELETE /api/admin/dict/items/{id}` | 字典项管理 |
| POST | `/api/admin/dict/refresh` | 刷新字典缓存 |
| GET | `/api/admin/audit-logs`、`/api/admin/logs/audit/export` | 操作日志查询 / 导出 |
| GET | `/api/admin/export/users` | 导出用户列表 |
| GET | `/api/admin/monitor/system`、`/api/admin/monitor/api-metrics`、`/api/admin/monitor/alerts` | 系统与接口监控 |
| GET | `/api/openapi.json`、`/api/swagger-ui/index.html` | OpenAPI 规范与 Swagger UI |

> 完整字段定义见 Swagger UI。集成测试会校验"文档里声明的每条路由都真实存在"。

## 权限码

每个 `/api/admin/*` 接口都要求一个权限码，缺失即 403（响应体带具体缺失的码）。
权限码保存在 `menus.permission` 列，对应 `type = 'button'` 的菜单行，
经既有的「角色-菜单」关联授权——**没有独立的权限表**，在「菜单管理」页即可调整。

| 模块 | 权限码 |
|------|--------|
| 用户 | `system:user:list` `system:user:create` `system:user:update` `system:user:delete` |
| 角色 | `system:role:list` `system:role:create` `system:role:update` `system:role:delete` |
| 菜单 | `system:menu:list` `system:menu:create` `system:menu:update` `system:menu:delete` `system:menu:grant` |
| 字典 | `system:dict:list` `system:dict:create` `system:dict:update` `system:dict:delete` `system:dict:refresh` |
| 日志 | `system:log:list` `system:log:export` |
| 监控 | `system:monitor:system` `system:monitor:api` `system:monitor:alert` `system:monitor:reset` `system:monitor:export` |
| 其他 | `system:export:user` `system:test:access` `system:validate:test` |

前后端共用同一套字符串：后端由 `src/model/permission.rs` 的 const 定义（单一数据源，
同时驱动启动种子），前端经 `GET /api/auth/permissions` 获取，并用
`v-permission="'system:user:create'"` 或 `<PermissionButton code="...">` 判定显隐。

```vue
<PermissionButton code="system:user:create">新建用户</PermissionButton>
<button v-permission="'system:user:delete'">删除</button>
```

权限码**不做缓存**：撤销后立即生效，不会出现"撤销了还能用一会儿"的窗口。
启动种子只给**新建的**权限码补授 admin，因此管理员被显式撤销的权限不会被重启悄悄恢复。

## 新增一个页面（菜单驱动）

前端不再有静态业务路由表，加页面只需两步：

1. 在 `frontend/src/views/` 下新增 `.vue` 文件，例如 `views/report/index.vue`
2. 在「菜单管理」里新增一条菜单：`path` = `/report`，`component` = `report/index`，
   并分配给它应有的角色

登录后 `/api/auth/menus` 会返回该菜单，前端据此注册路由并渲染侧栏；无需改前端路由代码。
`component` 解析不到页面文件时会跳过并告警（前端有契约测试提前拦截这类错配）。

## 测试与质量门禁

```bash
# 后端单元测试
cargo test

# 后端静态检查（CI 以 -D warnings 运行）
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings

# 后端集成测试（真实 Postgres + Redis，无需 Docker）
scripts/test_env.sh start
eval "$(scripts/test_env.sh env)"
cargo test --test api_integration -- --ignored --test-threads=1
scripts/test_env.sh stop

# 前端
cd frontend
pnpm lint         # ESLint 9 扁平配置
pnpm typecheck    # vue-tsc
pnpm test         # Vitest
pnpm build
```

另有两层**不进 CI**（需要真实 Chrome 与起好的服务），详见 `e2e/README.md`：

```bash
node e2e/run.mjs                # 真实 Chrome 端到端回归
node e2e/probe-write-guards.mjs # 授权探针：写入口清单从 OpenAPI 自动发现
```

探针的判定标准是：只持入口所需**最小权限码**的操作员，对权限高于自己的目标
发起写操作 → 必须被拒，且数据不得有任何变化。每个条目还要求 **admin 做同一件事
仍然成功**——只测拒绝侧的话，"把入口整个禁掉"也能全绿。新增写入口若未登记探测方式，
探针会直接报"未覆盖"，而不是静默漏掉。

集成测试覆盖：空库自动迁移、登录/登出与**旧令牌不复活**、错误口令 401、登录锁定 429、
普通用户越权 403、用户 CRUD 与角色投影、最后管理员保护、审计日志落库、
**角色化菜单树**、文档路由双向覆盖。

前端测试覆盖：请求缓存与本地存储、**菜单→路由转换**、动态路由注册/撤销、
菜单种子 component 与页面文件的一致性。

## 生产部署建议

1. **JWT_SECRET**：`openssl rand -base64 48` 生成，勿使用示例值（生产环境会拒绝启动）
2. **HTTPS**：Nginx 配置 SSL 证书；口令以明文经 TLS 提交，由服务端 Argon2 存储
3. **数据库**：使用托管数据库或强密码策略；迁移在应用启动时自动执行
4. **Redis**：设置 `requirepass` 并使用 ACL；Redis 不可用会直接返回 503
5. **反向代理**：仅在确实位于可信代理后时设置 `TRUST_PROXY_HEADERS=true`
6. **日志**：`RUST_LOG=info`，日志包含 `request_id` 便于链路定位
7. **监控**：`/api/health` 已做真实依赖探活，可直接用于编排健康检查
8. **备份**：定期备份 PostgreSQL 数据卷

## 从 v0.1 升级到 v0.2

- 迁移 `006` 会先把 `users.role` 回填进 `user_roles`，再删除该冗余列（保留存量权限）
- 口令模型由 `Argon2(sha256(明文))` 改为 `Argon2(明文)`；
  **存量账号在下次登录时自动透明升级**，无需重置密码
- 新增 `jti` 后，v0.1 签发的旧令牌无法通过校验，需重新登录
- 删除的配置项：`DATABASE_READ_URL`、`DB_READ_POOL_MAX_SIZE`、`CRYPTO_*`、`RSA_*`、
  `CAPTCHA_ENABLED`、`APP_SECRET`（这些能力此前均未真正生效）
- 字典读取接口路径由 `/api/admin/dict/{code}/items` 改为 `/api/dict/{code}/items`
- 移除端点：`PUT /api/admin/users/{id}/roles`（非事务且未被前端使用，
  请改用 `POST /api/admin/users/{id}/roles` 或用户更新的 `role` 字段）

## 从 v0.3 升级到 v0.4

- 迁移 `007` 为 `menus.permission` 建**部分唯一索引**。若库中已存在重复权限码，
  启动会明确报错（而不是抛看不懂的原始索引错误）；重复说明菜单被手工改坏，
  请在「菜单管理」页修正后重启——程序**不会**静默去重
- 启动时自动补齐 28 条权限码按钮行。菜单树本身仍只在 `menus` 表为空时写入，
  你对菜单的增删不会被覆盖
- `menus.permission` 从此**参与鉴权**，不再只是元数据：如果你的库里存在同名但语义不符的
  权限码，它会开始影响接口放行，请对照上表核对
- `/api/admin/*` 现按权限码鉴权。新装环境无需操作；**存量环境请确认 admin 角色
  已获得全部权限码**（「角色管理」→ 菜单授权），否则相关接口会返回 403
- 前端按钮显隐改为按权限码判定，不再依赖角色名。若你扩展了后端权限码，
  需同步 `frontend/src/constants/permission.ts`（有契约测试拦截漏改）

## 从 v0.4 升级到 v0.5

无新增迁移，**存量库直接重启即可**。但有四处行为变化需要注意：

- **授权接口不再吞错误**。`PUT /api/admin/roles/{id}/menus` 若收到非法菜单 ID，
  此前会静默跳过并返回「权限分配成功」，现在返回 **400**「提交的菜单 ID 不存在」。
  如果你有脚本在调这个接口，请先校验 ID
- **删除角色新增守卫**。内置角色（`admin` / `user`）与仍被用户占用的角色一律拒绝删除
 （400，message 里带占用用户数）。这是防止把系统改造成"无人能管理"的状态：
 角色种子只在 `roles` 表为空时写入，删掉不会重建
- **删除菜单改为依赖数据库级联**。原先手写递归删除会吞掉子节点的错误，
 可能留下 `parent_id` 指向已删父节点的孤儿菜单；现在由 `ON DELETE CASCADE`
 原子完成。行为上等价，但不会再产生脏数据
- **授权弹窗是全量覆盖**。打开授权、取消几个勾选并保存，会把该角色
 **未勾选**的既有授权一并撤销——这是接口一直以来的语义，现在界面终于如实呈现

## 从 v0.8 升级到 v0.9

无新增迁移，**存量库直接重启即可**。但有四处行为变化需要注意：

- **分配角色现在要求持有目标用户的权限码**。此前只查"授予什么"，不查"授予给谁"：
  只持 `system:user:update` 的角色能给一个纯 admin 账号追加角色。若你有脚本在调
  `POST /api/admin/users/{id}/roles`，现在会返回 **403**，message 里列出你没持有的码
- **分配角色会撤销目标用户的存量会话**。新授的权限立即生效，旧令牌不再可用
 （需重新登录）。重复追加同一角色是幂等的，不会踢掉该用户
- **删除角色新增授权天花板**。删角色 = 把这个角色承载的权限码从所有人身上撤走，
  与 v0.7.0 给"给角色授权"装的那道是同一件事的两面，因此现在同样要求你持有该码。
 **后果**：删带自定义码的角色必须先撤销授权；admin 自己造的码分发给别的角色后，
 admin 反而删不掉那个角色了（与 v0.8.0 的菜单删除同一性质）
- **令牌新增毫秒级 `iat_ms`**。升级前签发的旧令牌没有该字段，解析为 0，
  一定小于任何吊销时间点——方向是**失效**而非放行。也就是说**首次吊销会把存量令牌
 一并作废**，存量用户需要重新登录一次。这是安全的一侧，不是 bug

## License

This project is licensed under the [MIT License](LICENSE).
