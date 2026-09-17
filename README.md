# Axum Admin — 全栈管理系统

[![CI](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml/badge.svg)](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml)

基于 **Rust Axum** 后端 + **Vue 3** 前端的企业级全栈管理平台。

> v0.3.0 补齐了此前遗留的两项「声明与实现差距」：**OpenAPI 由代码生成**、
> **前端导航由后端菜单驱动**。变更明细见 [CHANGELOG.md](CHANGELOG.md)。

## 能力清单（与运行时一致）

| 能力 | 状态 | 说明 |
|------|------|------|
| JWT 认证 | ✅ | HS256；令牌带 `jti`，支持**单令牌注销**与**用户级会话吊销** |
| RBAC | ✅ | 角色唯一数据源为 `user_roles` 表；接口按角色拦截（admin / user） |
| 登录防护 | ✅ | 账号 + 客户端 IP 双维度失败计数，超阈值锁定；客户端 IP 默认取 TCP 真实来源 |
| 限流 | ✅ | 基于 Redis 的固定窗口限流（IP 维度） |
| 操作日志 | ✅ | 受保护路由写入 `audit_logs`（仅方法/路径/查询串，不记录请求体） |
| 数据字典 | ✅ | Redis 缓存 + 写操作真实失效；读取接口对**任意已登录用户**开放 |
| 菜单管理 | ✅ | 菜单是导航的唯一来源：`/api/auth/menus` 决定侧栏与前端动态路由 |
| 用户管理 | ✅ | 增删改查、批量删除、状态切换、重置密码；含"最后一个管理员"保护 |
| 系统监控 | ✅ | CPU/内存/磁盘、DB/Redis 状态、接口耗时统计（进程内，重启丢失） |
| Excel 导出 | ✅ | 用户列表、操作日志、系统信息 |
| OpenAPI 文档 | ✅ | utoipa 从 handler 注解与 DTO 派生生成；双向覆盖测试保证文档与路由同步 |
| 按钮级权限 | ⚠️ | 前端为**基于角色**的显隐（`PermissionButton`）；无独立权限码体系 |

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
| GET/POST | `/api/admin/roles`、`PUT/DELETE /api/admin/roles/{id}` | 角色管理 |
| GET/POST | `/api/admin/menus`、`PUT/DELETE /api/admin/menus/{id}` | 菜单管理（改动即时影响前端导航） |
| PUT | `/api/admin/roles/{id}/menus` | 角色-菜单关联 |
| GET/POST | `/api/admin/dict/types`、`PUT/DELETE /api/admin/dict/types/{id}` | 字典类型管理 |
| GET/POST | `/api/admin/dict/items`、`PUT/DELETE /api/admin/dict/items/{id}` | 字典项管理 |
| POST | `/api/admin/dict/refresh` | 刷新字典缓存 |
| GET | `/api/admin/audit-logs`、`/api/admin/logs/audit/export` | 操作日志查询 / 导出 |
| GET | `/api/admin/export/users` | 导出用户列表 |
| GET | `/api/admin/monitor/system`、`/api/admin/monitor/api-metrics`、`/api/admin/monitor/alerts` | 系统与接口监控 |
| GET | `/api/openapi.json`、`/api/swagger-ui/index.html` | OpenAPI 规范与 Swagger UI |

> 完整字段定义见 Swagger UI。集成测试会校验"文档里声明的每条路由都真实存在"。

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

## License

This project is licensed under the [MIT License](LICENSE).
