# Axum Admin — 全栈管理系统

[![CI](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml/badge.svg)](https://github.com/ZhongGheart/axum-api/actions/workflows/ci.yml)

基于 **Rust Axum** 后端 + **Vue 3** 前端的企业级全栈管理平台。

## 项目结构

```
axum-api/
├── src/                     # Rust 后端源码
│   ├── main.rs              # 入口：日志、配置、优雅关闭
│   ├── config/              # 配置层（DB、Redis、JWT、CORS、限流）
│   ├── router/              # 路由注册 + 全局中间件链
│   ├── controller/          # 控制器层（auth、user、role、rbac）
│   ├── service/             # 服务层（认证、RBAC）
│   ├── repository/          # 数据访问层（SQLx 纯SQL）
│   ├── model/               # 实体 + DTO 模型
│   ├── middleware/           # 中间件（JWT鉴权、限流、请求ID）
│   ├── error/               # 全局异常处理
│   └── utils/               # 工具（JWT、密码哈希、Redis）
├── frontend/                # Vue 3 前端源码
│   ├── src/
│   │   ├── api/             # Axios 请求层（对齐后端 controller）
│   │   ├── components/      # 通用组件（BaseTable、SearchForm等）
│   │   ├── views/           # 页面（login、register、system/）
│   │   ├── router/          # 路由配置 + 鉴权守卫
│   │   ├── stores/          # Pinia 状态管理
│   │   ├── directives/      # 自定义指令（v-permission）
│   │   └── utils/           # 工具（加密存储、SHA256、防抖等）
│   ├── nginx.conf           # Nginx 部署配置
│   └── Dockerfile           # 多阶段构建 -> nginx alpine
├── migrations/              # 数据库迁移 SQL
├── Cargo.toml
├── Dockerfile               # 后端多阶段构建 -> debian slim
└── docker-compose.yml       # 全栈编排（pg + redis + api + web）
```

## 技术栈

| 层级 | 技术 | 版本 |
|------|------|------|
| **后端** | Rust + Axum + Tokio | 1.82 / 0.8 |
| **数据库** | PostgreSQL + SQLx | 16 / 0.8 |
| **缓存** | Redis | 7 |
| **前端** | Vue 3 + TypeScript + Vite | 3.5 / 6 |
| **UI** | Naive UI | 2.41 |
| **部署** | Docker + Docker Compose + Nginx | — |

## 快速开始

### 环境要求

- Rust 1.82+
- Node.js 18+
- pnpm 最新版（`corepack enable && corepack prepare pnpm@latest --activate`）
- Docker & Docker Compose（可选）

### 本地运行（前后端分离）

#### 1. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env，修改 DATABASE_URL、JWT_SECRET、REDIS_URL
```

#### 2. 创建数据库

```bash
psql -U postgres -c "CREATE DATABASE axum_api;"
DATABASE_URL="postgres://postgres:password@localhost:5432/axum_api" sqlx migrate run
```

#### 3. 启动后端

```bash
# 开发模式（自动重载需安装 cargo-watch）
cargo run
# 或使用 RUST_LOG=debug cargo run 查看详细日志
```

#### 4. 启动前端

```bash
cd frontend
pnpm install
pnpm dev
# 默认监听 http://localhost:3000，自动代理 /api 到后端
```

#### 5. 访问

打开 `http://localhost:3000`，使用 `admin / admin123` 登录。

### Docker Compose 一键部署（推荐）

```bash
# 全栈启动（PostgreSQL + Redis + 后端 + 前端）
docker compose up -d

# 查看日志
docker compose logs -f api

# 停止
docker compose down

# 停止并删除数据卷
docker compose down -v
```

访问 `http://localhost`，Nginx 自动代理 `/api` 到后端。

### Docker 单容器运行

```bash
# 后端
docker build -t axum-api .
docker run -p 8080:8080 --env-file .env axum-api

# 前端
cd frontend
docker build -t axum-web .
docker run -p 80:80 axum-web
```

## API 文档

| Method | Path | Auth | 角色 | 说明 |
|--------|------|------|------|------|
| GET | `/api/health` | — | — | 健康检查 |
| POST | `/api/auth/register` | — | — | 用户注册 |
| POST | `/api/auth/login` | — | — | 登录（返回 JWT） |
| GET | `/api/auth/me` | JWT | — | 当前用户信息 |
| POST | `/api/auth/logout` | JWT | — | 登出（Redis 黑名单） |
| GET | `/api/admin/test` | JWT | admin | 权限测试 |
| GET | `/api/admin/users` | JWT | admin | 用户列表（分页） |
| POST | `/api/admin/users` | JWT | admin | 新建用户 |
| PUT | `/api/admin/users/:id` | JWT | admin | 更新用户 |
| DELETE | `/api/admin/users/:id` | JWT | admin | 删除用户 |
| GET | `/api/admin/roles` | JWT | admin | 角色列表 |
| GET | `/api/admin/users/:id/roles` | JWT | admin | 用户角色 |
| POST | `/api/admin/users/:id/roles` | JWT | admin | 分配角色 |

## 环境变量

### 后端（.env）

| 变量 | 必需 | 默认值 | 说明 |
|------|------|--------|------|
| `SERVER_HOST` | 否 | `0.0.0.0` | 监听地址 |
| `SERVER_PORT` | 否 | `8080` | 监听端口 |
| `DATABASE_URL` | **是** | — | PostgreSQL 连接字符串 |
| `REDIS_URL` | 否 | `redis://127.0.0.1:6379` | Redis 连接字符串 |
| `JWT_SECRET` | **是** | — | JWT 签名密钥 |
| `JWT_EXPIRATION_SECONDS` | 否 | `604800` | JWT 过期时间（秒） |
| `CORS_ALLOWED_ORIGINS` | 否 | `*` | CORS 允许的来源 |
| `DB_POOL_MAX_SIZE` | 否 | `20` | 数据库连接池大小 |
| `RATE_LIMIT_IP_MAX` | 否 | `100` | 单 IP 每分钟最大请求数 |

### 前端（.env.production）

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `VITE_API_BASE_URL` | `/api` | API 基础路径（同域部署无需修改） |

## 开发命令

```bash
# 后端
cargo build            # 编译
cargo test             # 运行测试
cargo clippy           # 代码检查
cargo fmt              # 代码格式化

# 前端
cd frontend
pnpm dev               # 开发服务器
pnpm build             # 生产构建
pnpm lint              # ESLint 检查
pnpm format            # Prettier 格式化
```

## 默认账号

| 用户名 | 密码 | 角色 |
|--------|------|------|
| `admin` | `admin123` | admin + user |
| 新注册用户 | 注册时设置 | user |

## 生产部署建议

1. **JWT 密钥**：使用 `openssl rand -base64 64` 生成强密钥
2. **HTTPS**：Nginx 配置 SSL 证书，前端使用 `https://`
3. **数据库**：使用托管数据库（RDS）或设置密码强策略
4. **Redis**：设置密码 `requirepass`，使用 ACL 控制
5. **日志**：配置 `RUST_LOG=info`，使用 JSON 格式输出到日志系统
6. **监控**：配置 `/api/health` 健康检查端点
7. **资源**：Docker 设置 CPU/内存限制
8. **备份**：定期备份 PostgreSQL 数据卷

## License

This project is licensed under the [MIT License](LICENSE).
