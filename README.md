# Axum Admin — 全栈管理系统

[![CI](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml/badge.svg)](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml)

基于 **Rust Axum** 后端 + **Vue 3** 前端的企业级全栈管理平台。

> v0.12.0 让"入参不合法"在任何端点都长一个样：此前 19 处 `Json<T>` 与 17 处 `Path<T>`
> 绕过 `AppError`，直接回 `400`/`415` + `text/plain`，而前端拦截器按 `message` 取文案——
> 纯文本那一种取不到，用户只看到一个空错误框。现全站改用 `ApiJson` / `ApiPath`，
> 统一为 `400` + `{code, message, data}`。
> 承重测试由 **OpenAPI 文档驱动**、遍历全路由实测响应形状：新增端点忘了迁移会当场变红。
> 顺带修掉一处实测发现的文档缺陷——`GET /api/admin/roles` 的 `page`/`page_size`
> 被 utoipa 标成了必填**路径**参数（它的 `ParameterIn::default()` 是 `Path`），
> 可路径模板里根本没有 `{page}`。
> v0.11.0 补上了安全追溯的基本盘：`/api/auth/login` 与 `/api/auth/register`
> 在 `public_routes` 里，此前**没有挂审计中间件**，因此登录成功、登录失败、
> 注册全部不进 `audit_logs`。这不是忘了挂——中间件依赖已认证用户，
> 而登录请求本来就没有，失败时更没有，所以这三处必须在 service 里显式写入。
> `action` 用语义值而非 `{METHOD} {path}`：登录成功与失败的方法路径完全相同，
> 只有 action 与身份有区分度，而"事后能否回答有没有人在爆破"正是这一版的初衷。
> 写入是**同步 await**：中间件那种 `tokio::spawn` 是为了不拖慢响应，
> 而审计静默丢失等于没审计。
> 同版新增自助改密与口令复杂度下限，并把"管理员建号 → 必须先改密"做成
> **受限令牌**而非"登录即踢下线"——`iat_ms` 升级已经让存量令牌作废过一次，
> 再来一次同类冲击是没必要的。存量用户因 `must_change_password` 默认 FALSE
> 完全不受影响。
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
| 限流 | ✅ | 基于 Redis 的固定窗口限流，**IP 与用户双维度**（已登录请求额外按 `user_id` 计一次） |
| 操作日志 | ✅ | 受保护路由写入 `audit_logs`（方法/路径/查询串 + **写操作变更摘要**；**不记录请求体**，口令与令牌不入库）；**登录成功/失败与注册也落审计**（`AUTH_LOGIN_SUCCESS`/`AUTH_LOGIN_FAILURE`/`AUTH_REGISTER`，带 `client_ip` 与失败原因）；支持按用户名/操作/状态码/时间范围**真筛选**；导出支持同一套条件，且**截断状态随响应头明示** |
| 变更追溯 | ✅ | 写操作由 handler 显式声明"改了什么"并落进 `result` 列：角色/用户/菜单**删除前先取名字**（行删掉后名字仍在审计里）、授权记录**授出与撤销的权限码差异**、状态切换记录前后状态；**口令一个字都不记**；失败的写操作不留摘要（不谎报"已授予"） |
| 数据字典 | ✅ | Redis 缓存 + 写操作真实失效；读取接口对**任意已登录用户**开放；**`status=disabled` 真的生效**（禁用项与禁用类型不进读取端点）；**`is_default` 靠部分唯一索引保证同一字典只有一个**；**「刷新缓存」真删 `dict:*` 并如实报出清了多少键** |
| 菜单管理 | ✅ | 菜单是导航的唯一来源：`/api/auth/menus` 决定侧栏与前端动态路由；**层级可调**（界面上能改上级、能摘成顶级）；**成环与悬空引用一律被拒**（自引用、挂到自己的子孙下、挂到不存在的上级均 400），且结构损坏的节点能通过 `/api/admin/menus/diagnostics` **被看见并救回** |
| 权限码 | ✅ | 30 个 `<模块>:<资源>:<动作>` 权限码存于 `menus.permission`（按钮型菜单），后端强制鉴权 + 前端按码判定；清空后可由清空者本人恢复 |
| 角色与授权 | ✅ | 角色增删改（**分页**）+ 菜单/权限码授权树；授权**要么完整成功、要么整体回滚**；内置角色不可删除、不可改名；自定义角色可直接分配给用户 |
| 用户管理 | ✅ | 增删改查（分页 + `keyword` 真搜索，另可按 `role` / `is_active` 筛选）、批量删除、状态切换、重置密码；**多角色分配**；含"最后一个管理员"保护 |
| 个人资料 | ✅ | `PUT /api/auth/profile` 自助改**展示名**与头像路径，字段级**三态**语义（不带 = 不改 / `null` = 清空 / 带值 = 设置），前端不必先读旧值再原样回写（那是典型的丢失更新） |
| 头像上传 | ✅ | `POST /api/auth/profile/avatar`（multipart，字段名 `file`）：**文件名由服务端生成 UUID**、扩展名由 MIME 白名单推导（绝不用客户端文件名拼路径）；替换时删旧文件，写库失败删新文件；`/uploads` 静态路由挂在鉴权之外。**落盘目录需挂卷**，见 `UPLOAD_DIR`。v0.27.0 起后端可切到 S3 兼容对象存储，见 `STORAGE_BACKEND` |
| 管理员解锁 | ✅ | `POST /api/admin/users/{id}/unlock` + **独立权限码 `system:user:unlock`**（不被 `system:user:update` 顺带放行）。清的是**用户名与邮箱两个桶**，刻意不碰 IP 桶——那是跨账号共享的，清了等于给爆破地址发新额度 |
| 在线会话 | ✅ | 登录时登记会话，`GET /api/admin/users/{id}/sessions` 列举、**单会话吊销** + 权限码 `system:session:manage`。登记靠 **JWT 自身 TTL** 自过期；**登记失败不放行登录**（漏掉一次登录记录比登录失败更危险：用户毫无察觉） |
| 批量导入用户 | ✅ | `POST /api/admin/users/import`（CSV，必需列 `username,email,password,roles`）：**逐行成败并带行号**（含表头上限），授权下界整批前置校验，`dry_run` 预览**绝不落库**；**口令一个字都不进审计** |
| 密码策略 | ✅ | 自助改密（验旧口令、改密后吊销全部会话）；长度 ≥ 8 且至少 2 类字符，**只在设置口令时校验、登录不校验**；前后端共用一份判定样例，由集成测试比对两侧结论 |
| 首次登录强制改密 | ✅ | 管理员新建/重置的用户须先改密，用**受限令牌**实现（只放行改密/登出/`/me`）；存量用户默认不受影响 |
| 系统监控 | ✅ | CPU/内存/磁盘、DB/Redis 状态、接口耗时统计（进程内，重启丢失） |
| Excel 导出 | ✅ | 用户列表、操作日志、系统信息；导出走与列表同一套筛选条件，超上限时**明示截断**而非静默砍数据 |
| 未知参数处理 | ✅ | 所有 query DTO 一律 `deny_unknown_fields`：拼错的参数直接 400 并指名字段，**不再静默丢弃**（v0.10.0 起） |
| 错误响应格式 | ✅ | **任何端点**的入参错误都是 `400` + `{code, message, data}`，含请求体与路径参数（v0.12.0 起）；原先 19 处 `Json` + 17 处 `Path` 会退回 `text/plain`，前端拦截器取不到 `message`，用户只看到一个空错误框 |
| OpenAPI 文档 | ✅ | utoipa 从 handler 注解与 DTO 派生生成；双向覆盖测试保证文档与路由同步 |
| 接口级权限码 | ⚠️ | 授权**完全由权限码决定**，没有角色闸门：`require_role("admin")` 及其 5 处调用已于 v0.5.0 PR-3 整体删除。持有 `system:user:list` 的自定义角色即可管理对应模块，因此"能进管理区"不等于"能做任何事"，越权靠**授权下界**（能授予的 ⊆ 自己已持有的）拦住 |

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
| `TOTP_ENCRYPTION_KEY` | 否 | 从 `JWT_SECRET` 派生 | **v0.25.0 新增**：AES-256-GCM 加密两步验证密钥用的主密钥（32 字节）。不设置时从 `JWT_SECRET` 加 `derived:` 前缀派生并告警。**已绑定 2FA 的部署应显式设置**：直接复用 `JWT_SECRET` 原文会让「轮换 JWT 密钥」静默地使所有已绑定用户的 2FA 永久失效，而本仓没有邮件通道，他们无法自助重绑 |
| `CORS_ALLOWED_ORIGINS` | 否 | `http://localhost:3000` | 逗号分隔；生产环境禁止 `*` |
| `DB_POOL_MAX_SIZE` | 否 | `20` | 数据库连接池大小 |
| `DB_CONNECT_TIMEOUT` | 否 | `10` | 连接超时（秒） |
| `RATE_LIMIT_IP_MAX` / `RATE_LIMIT_IP_WINDOW` | 否 | `100` / `60` | IP 限流阈值与窗口 |
| `RATE_LIMIT_USER_MAX` / `RATE_LIMIT_USER_WINDOW` | 否 | `30` / `60` | 已登录用户的限流阈值与窗口（与 IP 维度并存，任一超限即拒） |
| `TRUST_PROXY_HEADERS` | 否 | `false` | 是否信任 `X-Forwarded-For`（仅置于可信代理后时开启） |
| `LOGIN_MAX_FAILURES` | 否 | `10` | 登录失败锁定阈值。**v0.22.0 起降级为回落值**：管理员在「系统参数」页显式改过该参数后以参数表为准 |
| `LOGIN_FAILURE_WINDOW` | 否 | `300` | 登录失败计数窗口（秒）。**v0.22.0 起降级为回落值**，理由同上 |
| `AUDIT_LOG_RETENTION_DAYS` | 否 | `90` | 操作日志保留天数。**超期行会被后台任务无条件删除**，设为 `0` 关闭自动清理（改由运维自行处理） |
| `AUDIT_LOG_CLEANUP_INTERVAL_SECONDS` | 否 | `3600` | 清理任务的运行间隔（秒） |
| `AUDIT_LOG_CLEANUP_BATCH_SIZE` | 否 | `10000` | 单批删除行数上限：把长事务切碎，避免长时间持锁与 WAL 膨胀 |
| `AUDIT_LOG_CLEANUP_MAX_BATCHES` | 否 | `20` | 单轮清理最多执行多少批，删空即提前结束 |
| `METRICS_FLUSH_INTERVAL_SECONDS` | 否 | `5` | 内存指标缓冲的刷写间隔（秒）。传 `0` 会被兜底成 `1`，否则定时任务会空转刷 Redis |
| `METRICS_KEY_TTL_SECONDS` | 否 | `604800` | 指标在 Redis 里的存活时间（秒）。到期即丢弃，**调小会让监控页出现断点**，而不是只丢精度 |
| `METRICS_MAX_BUFFERED_ENDPOINTS` | 否 | `10000` | 单次刷写最多覆盖多少个不同端点，用来给内存占用封顶 |
| `UPLOAD_DIR` | 否 | `./uploads` | 头像落盘根目录。**容器部署必须挂卷**，否则重建容器会丢掉所有已上传的头像（compose 已配 `uploads:/app/uploads`） |
| `UPLOAD_MAX_FILE_SIZE` | 否 | `2097152` | 单张头像大小上限（字节，默认 2MB），超出回 413 |
| `STORAGE_BACKEND` | 否 | `local` | **v0.27.0 新增**：头像存储后端，`local`（落 `UPLOAD_DIR`）或 `s3`。不设置即 `local`，行为与 v0.26.0 完全一致 |
| `S3_BUCKET` | `s3` 时必填 | — | bucket 名。**程序不会替你建 bucket**，没建的表现是第一次上传头像报 500 |
| `S3_REGION` | 否 | `us-east-1` | 区域 |
| `S3_ENDPOINT` | 否 | AWS 默认 | 自建或第三方（MinIO / OSS / COS）**必须显式设置**，否则请求会打到 AWS 官方端点上去 |
| `S3_ACCESS_KEY_ID` | `s3` 时必填 | — | 访问密钥 ID |
| `S3_SECRET_ACCESS_KEY` | `s3` 时必填 | — | 访问密钥 |
| `S3_PUBLIC_BASE_URL` | 否 | 空（走站内代理） | 头像的对外访问基地址。**不设即由本进程代理读**：`avatar_url` 写成站内相对路径 `/uploads/{key}`，浏览器从本服务取（私有 bucket 不配 CDN 也能正常显示）。设了就变成浏览器直连对象存储/CDN，此时**直连地址必须在私有 bucket 上可匿名读**（即前面挂了 CDN 或开了公共读）。直连 S3 时要含 bucket 段（除非 CDN 抹掉了这一层） |
| `S3_KEY_PREFIX` | 否 | 空 | bucket 内的 key 前缀。key 本身已以 `avatars/` 开头，所以默认不再叠加（叠加会得到 `avatars/avatars/x.png`）。与别的应用共用 bucket 时用它隔开，如 `axum-api/prod` |

**关于 S3 后端的三件事，部署前请先读**：

1. **bucket 应该保持私有。** 把 bucket 设成公共读虽然更省事，
   但那些 URL 一旦泄露就永久可访问，且无法收回。
2. **`/uploads` 怎么工作，取决于有没有配 `S3_PUBLIC_BASE_URL`**（v0.28.0 起）：

   | 配置 | `avatar_url` 形态 | 谁提供字节 |
   |---|---|---|
   | 不设 | 站内相对路径 `/uploads/{key}` | **本服务代理读**（`GET /uploads/{key}`） |
   | 设了 | 对象存储/CDN 的绝对 URL | 浏览器直连，对象存储侧必须可匿名读 |

   v0.27.0 只有第二种，且**不设时会退回 `{endpoint}/{bucket}`** 拼一个直连地址——
   那在私有 bucket 上是 403（实测），于是"私有 bucket 又没 CDN"的部署方
   头像全裂，而文档给出的唯一解法是开公共读，两头堵死。
   现在不设即走代理模式，**私有 bucket 不配任何 CDN 也能正常显示头像**。
3. **切后端前已存的头像不会自动迁移。** 本地时期的 `/uploads/...` 地址在
   S3 后端下会由代理路由去桶里找同名 key，自然找不到；同理对象存储时期的
   绝对 URL 在切回本地后也不再有对应文件。换后端后让用户重传一次即可。
   清理方式是把旧目录或旧前缀下的对象按 `S3_KEY_PREFIX` 归档。

⚠️ **保留策略不是只写在文档里**：`GET /api/admin/audit-logs/retention` 会返回
当前部署的真实保留天数、现存最早一条日志的时刻，以及最近一次清理的
`cutoff_at` / 删除行数 / 是否撞上批数上限。系统日志页顶部也如实展示这些。
原因是 v0.13.0 起「改了什么」这一层进了 `audit_logs.result`，
删掉的不只是流水，而是复盘能力本身——而"日志为什么从某天起就查不到了"
与"那天什么都没发生过"在管理员眼里必须能被区分开。

⚠️ **v0.26.0 起了结构化的「涉及对象」**：写操作除了 `result` 文本，
还会写一行行 `audit_log_targets`（对象类型 / 对象 ID / 变更类型 / 当时的名字），
列表与导出都能按对象筛。**存量数据不回填**——从中文摘要反解 UUID 会张冠李戴
（`为用户 "X" 追加角色 "Y"（<uuid>）` 里那个 UUID 是用户的），
而错误的结构化数据比没有更危险：它看起来可信、会被直接引用，而答案是错的。
因此 v0.26.0 之前的历史行 `targets` 为空数组，界面与导出都如实说明
「该记录早于结构化上线」，而不是显示成空白让人以为那次操作没碰任何对象。

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
| POST | `/api/auth/login` | 登录（返回 JWT）。口令过期时返回**受限令牌**（`must_change_password=true`），只能改密 / 登出 / 查看自己 |
| POST | `/api/auth/2fa/verify` | **两步验证第二段**（v0.25.0）。口令已通过时用 `challenge_token` + 动态码/恢复码换正式令牌。公开端点——此时调用者尚未持有任何令牌 |
| GET | `/api/settings/password-policy` | **当前口令策略**（v0.22.0）。未登录可读，供注册页 / 改密页显示规则。**刻意不含**有效期与锁定阈值——把「账号多久被锁一次」暴露给未登录端点等于给爆破者一个可调的参数面板 |

### 需登录

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/auth/me` | 当前用户信息（含角色列表） |
| GET | `/api/auth/menus` | 当前用户可见的导航菜单树（前端动态路由与侧栏的数据源） |
| POST | `/api/auth/logout` | 注销**当前令牌** |
| GET | `/api/dict/{code}/items` | 读取字典项（任意已登录用户） |
| PUT | `/api/auth/profile` | **自助改资料**：展示名 / 头像路径，字段级三态 |
| POST | `/api/auth/profile/avatar` | **自助上传头像**（`multipart/form-data`，字段名 `file`），上传与设置一步完成 |
| GET | `/api/auth/2fa` | 当前用户的 2FA 状态（v0.25.0） |
| POST | `/api/auth/2fa/setup` | 开始绑定：生成密钥与 `otpauth://` URI（**此时尚未落库**） |
| POST | `/api/auth/2fa/enable` | 交出动态码确认绑定，返回 8 个一次性恢复码（**只此一次展示，库里仅存 SHA-256 摘要**） |
| POST | `/api/auth/2fa/disable` | 关闭 2FA，**需出示当前口令**（登录态本身可能来自被盗设备） |
| POST | `/api/auth/2fa/recovery-codes` | 重新生成恢复码，旧批次立即作废 |

### 仅 admin

| Method | Path | 说明 |
|--------|------|------|
| GET/POST | `/api/admin/users?page=&page_size=&keyword=&role=&is_active=` | 用户列表（分页 + 关键字搜用户名/邮箱，**另可按角色 / 启用状态筛选，两者同时给是 AND**） / 新建用户 |
| PUT/DELETE | `/api/admin/users/{id}` | 更新 / 删除用户 |
| POST | `/api/admin/users/batch-delete` | 批量删除（含最后管理员保护） |
| POST | `/api/admin/users/{id}/unlock` | **解锁**被登录失败计数锁定的账号（需 `system:user:unlock`） |
| GET | `/api/admin/users/{id}/sessions` | 该用户的**在线会话**列表（直接返回数组，不存在或无在线即空数组） |
| POST | `/api/admin/users/{id}/sessions/{jti}/revoke` | **单会话吊销**（需 `system:session:manage`） |
| POST | `/api/admin/users/import` | **CSV 批量导入用户**，请求体 `{csv, dry_run}`；逐行成败并带行号 |
| PUT | `/api/admin/users/{id}/status` | 启用 / 停用（停用即吊销会话） |
| POST | `/api/admin/users/{id}/reset-password` | 重置密码（并吊销会话） |
| GET/POST | `/api/admin/users/{id}/roles` | 查询 / 追加用户角色 |
| GET/POST | `/api/admin/roles?page=&page_size=`、`PUT/DELETE /api/admin/roles/{id}` | 角色管理（**列表已分页**；内置角色不可删除/改名；仍被用户占用的角色拒绝删除；角色名自动归一化，撞名 409） |
| GET/POST | `/api/admin/menus`、`PUT/DELETE /api/admin/menus/{id}` | 菜单管理（改动即时影响前端导航）；`parent_id` **三态**：不传=不改、`null`=摘成根、给 id=改上级 |
| PUT | `/api/admin/roles/{id}/menus` | 角色-菜单关联（**全量覆盖**；任一 ID 非法即整体 400） |
| GET | `/api/admin/menus?role_id={id}` | 某角色已授权的菜单树（授权弹窗的默认勾选值） |
| GET | `/api/admin/menus/diagnostics` | 菜单树结构诊断：走不到根、不在任何菜单树里的节点（成环 / 悬空引用）及其原因 |
| GET | `/api/admin/departments` | 部门树（v0.24.0，含 `level` 与 `path`） |
| GET | `/api/admin/departments/flat` | 部门扁平列表，供下拉选择 |
| GET | `/api/admin/departments/{id}/users` | 该部门下的用户。**部门不存在返回 200 + 空数组**，与用户会话列表同规则 |
| POST | `/api/admin/departments/{id}/move` | 移动部门。**不能移到自己或自己的子孙下面** |
| GET/POST | `/api/admin/dict/types`、`PUT/DELETE /api/admin/dict/types/{id}` | 字典类型管理 |
| GET/POST | `/api/admin/dict/items`、`PUT/DELETE /api/admin/dict/items/{id}` | 字典项管理 |
| POST | `/api/admin/dict/refresh` | 刷新字典缓存 |
| GET | `/api/admin/settings` | **系统参数列表**（v0.22.0）。参数名 / 类型 / 取值范围 / 默认值 / 说明 / **消费方**全部由服务端下发，前端不硬编码清单 |
| PUT | `/api/admin/settings/{key}` | 修改单个参数（写入前校验类型与范围，并检查跨字段约束；越界或与另一参数冲突即 400） |
| POST | `/api/admin/settings/{key}/reset` | 复位参数。有部署配置兜底的参数会**交还控制权给环境变量**，而不是钉死在代码默认值上 |
| POST | `/api/admin/settings/refresh-cache` | 清理参数缓存 |
| GET | `/api/admin/audit-logs?page=&username=&action=&status_code=&start_time=&end_time=&target_type=&target_id=&target_key=` | 操作日志查询（分页 + 真筛选）。**v0.26.0 起支持按「涉及对象」筛**：`target_type` + `target_id` 回答"谁改过 role:3 的权限"，`target_key` 用于系统参数（主键是字符串，没有 UUID）。看不懂的 `target_type` 回 400 而不是静默查不到 |
| GET | `/api/admin/logs/audit/export`（同上筛选参数） | 操作日志导出；响应头带 `x-export-row-count` / `x-export-truncated` / `x-export-max-rows`。**v0.26.0 起多了「涉及对象」列** |
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
| 系统参数（v0.22.0） | `system:setting:list` `system:setting:update` |
| 部门（v0.24.0） | `system:dept:list` `system:dept:create` `system:dept:update` `system:dept:delete` |
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
# ⚠️ 两组都要跑。只跑 `--ignored` 那组会漏掉 8 条——v0.12.0 的门禁正是这么漏的，
#    当时"集成 103 全绿"里含一条实际为红的用例。--test-threads=1 不可省：
#    这些用例共用同一个测试库，并发跑会互相污染。
cargo test --test api_integration -- --test-threads=1              # 非 ignored 组
cargo test --test api_integration -- --ignored --test-threads=1   # 需要真实依赖的组
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

⚠️ **e2e 必须配一套专用环境变量**，详见 `e2e/README.md` 的前置段。其中最容易漏的是：

```bash
export RATE_LIMIT_IP_MAX=100000   # 默认 100 次/分，一轮 e2e 必然打满
export RATE_LIMIT_USER_MAX=100000
```

漏了这两行，后果**不是**"限流测试失败"——429 会顺着断言扩散成七八条红
（"无控制台错误""令牌已被清除""被弹回登录页"…），看起来像一堆功能坏了。
限流本身由集成测试覆盖，e2e 不需要它挡路。

另：e2e 与集成测试**共用同一个库和同一个 Redis**（都是 `scripts/test_env.sh` 起的
`axum_api_test`），两组连着跑会互相污染（登录失败计数、限流窗口、上轮遗留的夹具）。
真要连续跑，先重启测试环境。这是**已知的隔离缺口**，尚未修。

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

## 从 v0.9 升级到 v0.10

无新增迁移，**存量库直接重启即可**。但有三处行为变化需要注意：

- **`GET /api/admin/roles` 的返回值变了（破坏性）**。从裸数组改为分页对象：

  ```diff
  - { "code": 200, "data": [ { "id": "…", "name": "admin" } ] }
  + { "code": 200, "data": { "items": [ … ], "total": 34, "page": 1, "page_size": 10, "total_pages": 4 } }
  ```

  取数改成 `res.data.items`。注意排序是 `created_at ASC`（最早的角色在前），
  **按名字查某个角色时不能只看第一页**——本仓库的授权探针最初就踩了这个坑，
  把"角色还在"误判成"被越权删了"，那样的假红比不测更糟

- **多传 query 参数会开始报 400**。所有 `/api/admin/*` 的 query DTO 现在
  `deny_unknown_fields`，拼错的字段名直接 400 并在 message 里指名。
  这对原先依赖"多传参数被静默忽略"的客户端是**行为变更**——但那正是
  本版要消灭的失效方式：`keyword` 当初就是这样悄无声息失效的。
  若你只是想在筛选栏上加条件，请同步改后端 DTO（前端字段名必须与 DTO 逐字对齐，
  有契约测试守着）

- **日志导出会带上截断状态**。`/api/admin/logs/audit/export` 的响应头新增
  `x-export-row-count` / `x-export-truncated` / `x-export-max-rows`。
  上限 10000 行**保留**（日志表无限增长，没有上限迟早 OOM），
  变的是它不再静默：触顶时 `x-export-truncated: true`

另有一处**界面变化**：组件示例页的上传控件已移除。它原先写死
`:action="'/api/upload'"`，而该端点在路由表里不存在。`BaseUpload` 组件本身
仍保留在 `frontend/src/components/common/BaseUpload.vue`，等后端真有了上传端点再接回。

## 从 v0.17 升级到 v0.18

无新增迁移，**存量库直接重启即可**。有四处行为变化需要注意：

- **中文用户名的长度上限从"字节"改成"字符"**。此前 `validate_username` 用
  `str::len()`（字节）判定，而报错文案说的是"3-50 个**字符**"——
  文案与判据说的不是一回事。后果是 17 个汉字（51 字节 / 17 字符）会被拒，
  管理员看到的是一句对着合规用户名的报错。
  而 `users.username` 是 `varchar(50)`，Postgres 按**字符**计，列本来就收得下，
  也就是说这条限制比数据库更严，凭空砍掉了三分之二的中文用户名容量。
  方向是**放宽**：原先被拒的中文用户名现在可以注册，存量数据不受影响

- **注册页与管理员建号对话框不再接受弱口令**（前端侧）。
  这两处的 `min: 6` 是 v0.10 时代的化石——v0.11.0 已把策略收紧为
  "至少 8 位 + 两类字符"，但只迁移了改密页。现在三处同源。
  **如果你有脚本在调建号接口**，前端拦截不影响它，后端行为在 v0.11.0 就已改变

- **登录页的标识符输入框上限从 50 改成 255**。该字段收的是"用户名**或邮箱**"，
  而 `users.email` 是 `varchar(255)`。原先写死 50 的后果是
  **持有长邮箱的合法用户在自己的登录页上敲不进自己的邮箱**。
  方向是**放宽**

- **`GET /api/openapi.json` 里 `RegisterRequest` 的字段描述变了**。
  `password` 原先写"至少 6 个字符"（v0.11.0 起就是错的），现改为现行策略；
  `username` / `email` 补上了此前漏写的字符集与长度上限。
  这三条都不是新增限制，只是让文档不再教人用会被拒绝的规则。
  若你的客户端生成代码时读取这段描述做校验，需要重新生成

## 从 v0.19 升级到 v0.20

两条迁移（`014_user_profile_fields` / `015_widen_audit_action`）均由启动时自动执行，
**存量库直接重启即可**。需要留意四处：

- **新增两个权限码，且已自动授予 admin**：`system:user:unlock` 与
  `system:session:manage`。种子只给**本次新建**的权限码授权，
  所以管理员日后在「菜单管理」页撤销的授权不会被下次启动悄悄恢复。
  **自定义管理角色需要手动补这两个码**，否则解锁与吊销会话会 403

- **`users` 表新增两列，存量行均为 `NULL`**。`display_name` 为 NULL 时界面回退显示用户名，
  与 v0.20.0 之前一致，不会有视觉跳变

- **`audit_logs.action` 列宽从 `VARCHAR(100)` 放宽到 `VARCHAR(512)`。**
  此前只要请求路径略长，审计记录就会**整条写入失败且不报错**——
  写库在 `tokio::spawn` 里，失败只留一行 `tracing::warn!`，请求照常返回 200。
  表现是"这个操作没有审计记录"。v0.20.0 新增的会话吊销路径第一次越过 100 字符这条线

- **`UPLOAD_DIR`（默认 `./uploads`）需要挂卷，否则重建容器会丢头像。**
  `docker compose up` 已自动配置 `uploads:/app/uploads` 命名卷；
  若是自行编排，务必挂到 `/app/uploads` 且属主要与容器内用户一致
  （镜像里已预建该目录并设好属主，卷挂到**已存在**的路径会继承它）

**行为变化**：`GET /api/admin/users/{id}/sessions` 对**不存在的用户**返回 `200` + 空数组，
而不是 404。这是为了与 `GET /api/admin/users/{id}/roles` 保持一致——
同一个 `{id}` 在两个"查这个人的附属信息"的端点上不该给出两种相反的答案。
副作用是：一个写错或已删除的 id 与"这个人确实没在线"在响应上分不开。

## 从 v0.21 升级到 v0.22

**有数据库迁移**（`016_system_settings.sql`），**新增 5 个 API**、**2 个权限码**。
默认配置下**没有任何用户行为变化**——所有新参数的默认值都等于 v0.21 的硬编码常量。

### 迁移做了什么

1. 新增表 `system_settings`（7 个参数种子行）
2. `users` 新增列 `password_changed_at`，并**回填成 `created_at`**

第 2 条回填是刻意的，且方向不能反：

- 回填成 `created_at` → 一旦管理员把「口令有效期」从 `0` 调成非 `0`，
  存量口令按其设置时刻算起，多数会被要求改密。**这正是启用该策略的目的。**
- 回填成 `now()`（或干脆留 `NULL`）→ 所有存量用户在管理员开启策略后
  又白白多活一个完整周期，**策略形同虚设**。

默认 `expiry_days = 0`（永不过期），所以**默认部署下没有任何人被影响**。
迁移注释里写明了这一点，因为「一开策略全员被要求改密」是很强的行为，
运维需要提前知情而不是升级后才发现。

### 环境变量降级为回落值

`LOGIN_MAX_FAILURES` 与 `LOGIN_FAILURE_WINDOW` 仍然有效，但语义变了：

> **管理员在「系统参数」页显式改过 → 用参数表的值；没人改过 → 用环境变量的值。**

这个顺序是本版踩过坑之后定的，两个方向都会出事：

- 参数表无条件优先 → 部署时设的环境变量被**静默忽略**。
  实测：靠 `LOGIN_MAX_FAILURES=3` 构造低阈值的部署里，种子值 10 直接盖掉它，
  于是「失败 3 次应被锁定」变成 200，**日志里一个字都没有**。
- 环境变量无条件优先 → 管理员在界面上改了参数，重启后又变回去，
  「写入成功」却永远不生效。

参数页会显示每个值的来源（`管理员设置` / `部署配置` / `代码默认值`）。
「复位」一个有部署配置兜底的参数，会把控制权**交还环境变量**，
而不是把参数钉死在代码默认值上。

**如果你此前靠环境变量调这两个值**：升级后行为不变，无需改动。

### 新增权限码需要授权

`system:setting:list`（读）与 `system:setting:update`（改 / 复位 / 清缓存）。
**默认 `admin` 角色自动拥有**；其他角色若需要访问参数页，请手动授权。

存量库也会自动补齐这两个码与 `/system/setting` 页面菜单——
新增的 `backfill_late_added_menus` 专门解决「页面菜单只在 `menus` 表为空时写入」
导致已存在部署永远拿不到新菜单、进而新权限码解析不到父菜单而无法授权的问题。

### 用户可见的两处行为

- **口令过期的用户仍能登录**，但拿到的是**受限令牌**：只能改密 / 登出 / 查看自己。
  这是刻意设计的——本仓没有邮件通道，拒绝登录等于账号永久锁死，
  而用户连「为什么被拒」都看不到。
- **注册页与改密页的口令提示现在跟着服务端走**。管理员调高门槛后，
  界面提示会同步变化，而不是继续印着旧数字。
  （此前界面印的是写死的 8 位 / 两类，与后端实际执行的规则会分叉。）

### 升级步骤

```bash
# 1. 后端（含迁移，MIGRATE_ON_STARTUP=true 时自动执行）
cargo build --release

# 2. 前端
cd frontend && pnpm install && pnpm build
```

迁移是**向后兼容的**：先跑新二进制、后跑新前端没有顺序要求。
唯一的注意事项是别在旧二进制还活着时用新前端改参数——
旧版本不认识 `system_settings`，改了也不会生效。

## 从 v0.20 升级到 v0.21

**无数据库迁移，无 API 变化，无环境变量变化，无新增权限码。**
后端 Rust 一行未改，升级只需重新构建前端（`pnpm build` 后替换静态资源）。

唯一值得提前告知的用户可见变化：**首页现在真的会显示内容**。
此前登录后首页是一片空白，那是缺陷不是设计——详见 CHANGELOG 的
"界面在，但首页从来没出现过"。若你的用户在 v0.20 上报告过"首页是空的"，
本版即修复。

界面上另有三处行为变化，都是修正而非新增：

- 主色由 `#2080f0` 改为 `#2b5fd9`（偏青靛蓝）。自定义主题请改
  `frontend/src/stores/app.ts` 里的主色常量
- 侧栏菜单图标改为**按路径兜底**：后端给多个菜单种了同一个 `icon='settings'`，
  此前侧栏会出现六排一模一样的齿轮。现在按菜单路径回退到不同图标。
  **只改显示，不动菜单表的 `icon` 字段**（那是管理员的数据）
- 登录页标签文案由"记住密码"改为**"记住用户名"**——实现本来就只记用户名

## 从 v0.22 升级到 v0.23

**一次迁移，无 API 变化，无环境变量变化，无新增权限码。**

迁移 `017` 往 `system_settings` 插两行种子
（`security.registration.enabled = true`、`security.session.max_concurrent = 0`），
**不改表结构**。`MIGRATE_ON_STARTUP=true` 时自动执行。

三个值得提前告知的用户可见变化：

1. **管理员现在可以关掉公开注册。** 在「系统参数」页的**注册准入**分组里
   把「开放注册」拨到关，`/api/auth/register` 会立即开始返回 403。
2. **用户现在能自己管理登录会话。** 个人中心新增「登录会话」卡片，
   可以列出在线会话、下线单个设备、一键下线其他所有设备。
3. **管理员现在能限制并发会话数。** 在「系统参数」页的**会话**分组里
   把「并发会话上限」从 0 改成 N，同一账号最多同时在线 N 个会话。

六点必须提前告知，否则会被当成缺陷上报：

- **注册默认仍然是开放的。** 这是刻意的：本参数出现之前注册就是无条件开放的，
  默认改 `false` 会让一次常规发版突然关掉所有存量部署的注册入口。
  需要关注册的部署请显式去关。
- **关闭注册不影响已注册用户登录。** 两条路径相互独立。
- **关闭注册不影响管理员建号。** 管理员在用户管理里创建用户走的是另一条路径。
- **并发会话上限默认是 0（不限制）。** 同理，此前同一账号可在任意多设备同时在线，
  默认非 0 会让一次常规发版突然只允许有限设备登录。
- **并发上限不踢已在线的会话。** 它约束的是"还能不能新开一个会话"；
  要踢设备请用个人中心的「下线其他所有设备」。
- **自助会话端点不在受限令牌白名单里。** 待改密的用户先改口令，
  与改资料、传头像同一原则。

被拒的注册会落审计（`result="注册已关闭"`），
所以「关闭注册后还有谁来撞过注册口」在审计日志里仍然查得到。

## 从 v0.23 升级到 v0.25

**两次迁移（`018` + `019`），无 API 破坏性变更，新增一个可选环境变量。**
`MIGRATE_ON_STARTUP=true` 时自动执行。

迁移 `018` 新增 `departments` 表并给 `users` 加 `dept_id`；
迁移 `019` 给 `users` 加 `totp_secret_enc` / `totp_enabled_at` 两列
并新增 `user_two_factor_recovery` 表。**两处都不改任何存量行**——
存量用户的 `dept_id` 与 `totp_enabled_at` 均为 NULL，
即「无部门」与「未启用 2FA」，行为与升级前逐字一致。

### 新增权限码需要授权

`system:dept:list` / `create` / `update` / `delete` 四个码随启动种子补授 admin。
**若你显式撤销过 admin 的部门权限，重启不会悄悄恢复**（与既有权限码同一原则）。
其他角色需要在「菜单管理」里单独授权，否则看不到部门管理页。

### 六点必须提前告知，否则会被当成缺陷上报

- **注册默认仍然开放，并发会话上限默认仍然是 0。** 这两项是 v0.23.0 的行为，
  本版未改动默认值。
- **2FA 默认对所有用户关闭。** 需要用户在个人中心主动绑定，
  管理员无法代为开启——代开意味着管理员要经手用户的认证器。
- **绑定时恢复码只展示一次。** 库里只存 SHA-256 摘要，无法二次查看。
  弹窗因此禁用了遮罩点击与 ESC，并提供复制与下载两条出路。
- **关闭 2FA 会连带作废全部恢复码**，由数据库触发器保证，不走应用层。
- **同一动态码不能用第二次。** 按 RFC 6238 §5.2 用 Redis Lua 原子占位，
  误触发后需要等下一个 30 秒窗口。
- **`TOTP_ENCRYPTION_KEY` 不设置也能跑**，但会告警。已绑定 2FA 的部署
  **应当显式设置**：从 `JWT_SECRET` 派生时，轮换 `JWT_SECRET` 会让所有
  已绑定用户的 2FA 永久不可解，而本仓没有邮件通道，他们无法自助重绑。

### 升级步骤

1. 备份数据库（迁移 `019` 加列并建表，虽然不改存量行，仍建议先备）。
2. 可选：设置 `TOTP_ENCRYPTION_KEY`（32 字节随机串），**在首次绑定之前设好**。
3. 升级后端，`MIGRATE_ON_STARTUP=true` 会自动跑 `018` 与 `019`。
4. 升级前端，进入「系统管理 → 部门管理」确认页面可见；
   若不可见，检查该角色的菜单授权（见上一节）。

## License

This project is licensed under the [MIT License](LICENSE).
