# Axum Admin — 前端管理面板

基于 Vue 3 + TypeScript + Naive UI 的后台管理系统前端。

## 技术栈

| 类别 | 技术 |
|------|------|
| 框架 | Vue 3 (Composition API) |
| 语言 | TypeScript |
| 构建 | Vite 6 |
| UI 库 | Naive UI |
| 状态管理 | Pinia |
| 路由 | Vue Router 4 |
| HTTP | Axios |
| 代码规范 | ESLint + Prettier |

## 目录结构

```
src/
├── api/               # 网络请求层
│   ├── index.ts       # Axios 实例 + 拦截器（重试/loading）
│   ├── auth.ts        # 认证 API
│   ├── user.ts        # 用户管理 API
│   └── role.ts        # 角色管理 API
├── assets/            # 全局样式（主题/过渡/CSS变量）
├── components/
│   └── common/        # 通用组件（BaseTable/SearchForm/PageLoading/PermissionButton）
├── directives/        # v-permission 权限指令
├── layouts/           # 空白布局
├── router/            # 路由 + 鉴权守卫（Token过期/角色检测）
├── stores/            # Pinia：app/user/permission
├── utils/             # 工具（storage/crypto/message/perform）
├── types/             # 全局类型扩展
└── views/
    ├── login/         # 登录页
    ├── register/      # 注册页
    ├── home/          # 首页
    ├── system/        # 系统管理（用户管理/角色管理）
    └── error/         # 404
```

## 环境要求

- Node.js >= 18
- pnpm（推荐）

```bash
# 安装 pnpm
corepack enable && corepack prepare pnpm@latest --activate
```

## 快速开始

```bash
# 进入前端目录
cd frontend

# 安装依赖
pnpm install

# 开发环境运行（端口 3000，代理 /api 到 localhost:8080）
pnpm dev

# 生产构建
pnpm build

# 预览构建产物
pnpm preview
```

## 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `VITE_API_BASE_URL` | API 基础路径 | `/api` |
| `VITE_APP_TITLE` | 应用标题 | `Axum Admin` |

开发环境通过 Vite proxy 将 `/api` 代理到后端 `http://localhost:8080`。
生产环境通过 Nginx 反向代理（参考 `nginx.conf`）。

## 构建优化

- **分包策略**：manualChunks 将 naive-ui/vue/axios 拆为独立 vendor
- **双重压缩**：gzip + brotli（需 Nginx 配置相应模块）
- **CSS 压缩**：esbuild minifier
- **缓存策略**：`/assets/` 设置 `immutable` 一年缓存

## Nginx 部署

提供 `nginx.conf`，适配 SPA + API 反向代理：

```bash
docker build -t axum-web .
docker run -p 80:80 axum-web
```

或直接 Nginx 部署：

```bash
pnpm build
# 将 dist/ 目录和 nginx.conf 复制到服务器
# 修改 nginx.conf 中 upstream backend 地址
```

## 代码检查

```bash
pnpm lint          # ESLint
pnpm format        # Prettier
pnpm build         # vue-tsc 类型检查 + Vite 构建
```

## 默认账号

| 用户名 | 密码 | 说明 |
|--------|------|------|
| `admin` | `admin123` | 超管（可访问系统管理） |
| 新注册用户 | 自设 | 普通用户（仅首页） |
