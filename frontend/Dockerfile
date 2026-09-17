# ============================================
# 前端 Dockerfile — 多阶段构建
# ============================================
# 阶段一：依赖安装与编译
# ============================================
FROM node:22-alpine AS builder

WORKDIR /app

# 安装 pnpm
RUN corepack enable && corepack prepare pnpm@10.17.1 --activate

# 复制依赖配置文件
COPY package.json pnpm-lock.yaml ./

# 安装依赖（利用 Docker 缓存）
RUN pnpm install --frozen-lockfile

# 复制源码
COPY . .

# 生产构建
RUN pnpm build

# ============================================
# 阶段二：运行镜像（极简 nginx alpine）
# ============================================
FROM nginx:1.27-alpine AS runtime

# 从构建阶段复制编译产物
COPY --from=builder /app/dist /usr/share/nginx/html

# 复制 Nginx 配置
COPY --from=builder /app/nginx.conf /etc/nginx/conf.d/default.conf

# 健康检查
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://localhost:80/api/health || exit 1

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
