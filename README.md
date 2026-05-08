# Axum API — 生产级 Rust 后端项目

基于 [Axum](https://github.com/tokio-rs/axum) 框架构建的生产级 RESTful API 后端，采用纯 Rust 实现。

## 技术栈

| 类别 | 技术 | 说明 |
|------|------|------|
| **Web 框架** | Axum 0.8 | 基于 Tokio 的高性能异步 Web 框架 |
| **异步运行时** | Tokio | Rust 生态标准异步运行时 |
| **数据库** | PostgreSQL + SQLx | 纯 SQL 方式，编译期 SQL 校验 |
| **序列化** | serde + serde_json | 高性能 JSON 序列化/反序列化 |
| **错误处理** | thiserror + anyhow | 自定义错误枚举 + 便捷错误传播 |
| **日志追踪** | tracing + tracing-subscriber | 结构化日志，span 追踪 |
| **配置管理** | dotenvy | 从 `.env` 文件加载配置 |
| **密码加密** | Argon2 | OWASP 推荐的密码哈希算法 |
| **JWT 认证** | jsonwebtoken | 无状态 JWT 登录鉴权 |
| **CORS/中间件** | tower-http | CORS、Trace 等中间件 |

## 项目结构

```
src/
├── main.rs            # 应用入口：启动服务器、初始化日志
├── config/            # 配置层：从 .env 加载配置
├── router/            # 路由层：注册 API 路由分组
├── controller/        # 控制器层：处理 HTTP 请求
├── service/           # 服务层：业务逻辑
├── repository/        # 数据访问层：SQLx 数据库操作
├── model/             # 模型层：数据实体与响应结构
├── middleware/         # 中间件层：JWT 鉴权、权限拦截
├── error/             # 错误层：全局异常处理
└── utils/             # 工具层：JWT、密码哈希
```

### 分层调用链

```
HTTP Request
  → Router (路由匹配)
    → Middleware (JWT 鉴中间件)
      → Controller (参数校验)
        → Service (业务逻辑)
          → Repository (SQLx 数据库操作)
            → PostgreSQL
```

## API 端点

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/auth/register` | No | 用户注册 |
| POST | `/api/auth/login` | No | 用户登录，返回 JWT |
| GET | `/api/auth/me` | Yes | 获取当前用户信息 |
| GET | `/api/health` | No | 健康检查 |

### 统一响应格式

```json
{
  "code": 200,
  "message": "success",
  "data": { ... }
}
```

## 快速开始

### 前置条件

- Rust 1.82+
- PostgreSQL 14+
- Docker（可选，用于容器部署）

### 1. 克隆项目

```bash
git clone <your-repo-url> axum-api
cd axum-api
```

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env 文件，修改数据库连接等配置
```

### 3. 创建数据库

```bash
# 登录 PostgreSQL 并创建数据库
psql -U postgres
CREATE DATABASE axum_api;
\q

# 运行数据库迁移
DATABASE_URL=postgres://postgres:password@localhost:5432/axum_api sqlx migrate run
```

### 4. 编译并运行

```bash
# 开发模式
cargo run

# 或使用 Release 模式
cargo run --release
```

### 5. 验证服务

```bash
# 健康检查
curl http://localhost:8080/api/health

# 注册用户
curl -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"username":"testuser","email":"test@example.com","password":"password123"}'

# 登录
curl -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"testuser","password":"password123"}'

# 获取当前用户信息（使用登录返回的 token）
curl http://localhost:8080/api/auth/me \
  -H "Authorization: Bearer <your-jwt-token>"
```

## Docker 部署

### 使用 Docker Compose（推荐）

创建 `docker-compose.yml`：

```yaml
version: "3.9"
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: axum_api
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: password
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data

  api:
    build: .
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://postgres:password@postgres:5432/axum_api
      JWT_SECRET: "change-this-to-a-random-secret"
    depends_on:
      - postgres

volumes:
  pgdata:
```

```bash
docker compose up -d
```

### 单容器构建

```bash
docker build -t axum-api .
docker run -p 8080:8080 --env-file .env axum-api
```

## 配置说明

参考 `.env.example` 文件：

| 变量 | 必需 | 默认值 | 说明 |
|------|------|--------|------|
| `SERVER_HOST` | 否 | `0.0.0.0` | 监听地址 |
| `SERVER_PORT` | 否 | `8080` | 监听端口 |
| `DATABASE_URL` | **是** | - | PostgreSQL 连接字符串 |
| `JWT_SECRET` | **是** | - | JWT 签名密钥 |
| `JWT_EXPIRATION_SECONDS` | 否 | `604800` | JWT 过期时间（秒） |
| `CORS_ALLOWED_ORIGINS` | 否 | `*` | CORS 允许的来源 |

## 日志配置

通过 `RUST_LOG` 环境变量控制日志级别：

```bash
# 调试模式
RUST_LOG=debug cargo run

# 只显示当前模块的 info 及以上级别
RUST_LOG=info cargo run

# 禁用第三方库的日志
RUST_LOG=info,axum=warn,tower_http=warn cargo run
```

## 生产部署建议

1. **JWT 密钥**：使用足够长的随机字符串（建议 64 字节以上）
2. **数据库连接池**：根据并发量调整 SQLx 连接池大小
3. **日志收集**：配置 JSON 格式日志输出到日志收集系统
4. **健康检查**：配置负载均衡器的健康检查端点
5. **HTTPS**：使用反向代理（如 Nginx）终止 TLS
6. **资源限制**：Docker 部署时设置 CPU/内存限制

## 开发命令

```bash
# 编译
cargo build

# 运行测试
cargo test

# 检查代码（需要先安装 clippy）
cargo clippy -- -D warnings

# 格式化代码
cargo fmt
```
