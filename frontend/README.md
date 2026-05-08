# Axum Admin — 前端管理面板

基于 Vue 3 + TypeScript + Naive UI 构建的后台管理系统前端。

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
| 图标 | Iconify |
| 代码规范 | ESLint + Prettier |

## 目录结构

```
src/
├── api/            # 网络请求层（对齐后端 controller）
│   ├── index.ts    # Axios 实例 + 拦截器
│   └── types/      # 全局 TS 类型（复刻后端结构体）
├── assets/         # 静态资源
├── components/     # 通用组件
├── layouts/        # 布局组件
├── router/         # 路由配置 + 守卫
├── stores/         # Pinia 状态管理
├── utils/          # 工具函数
└── views/          # 页面视图
```

## 环境要求

- Node.js >= 18
- pnpm（推荐）或 npm

## 快速开始

```bash
# 安装依赖
pnpm install

# 开发环境运行（端口 3000，代理 API 到 localhost:8080）
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
生产环境建议通过 Nginx 反向代理或同域部署。

## 部署

```bash
# 构建
pnpm build

# 产物在 dist/ 目录下，部署到 Nginx
# 配置参考：
# location /api {
#     proxy_pass http://backend:8080;
# }
# location / {
#     root /path/to/dist;
#     try_files $uri $uri/ /index.html;
# }
```
