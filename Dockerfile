# ============================================
# Dockerfile — 多阶段构建
# ============================================
# 阶段一：编译构建
# ============================================
FROM rust:1.82-slim-bookworm AS builder

# 安装编译依赖
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# 创建工作目录
WORKDIR /app

# 先复制 Cargo.toml 和 Cargo.lock 以缓存依赖层
COPY Cargo.toml ./

# 创建空的 main.rs 以构建依赖缓存
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
RUN rm -rf src

# 复制完整源代码
COPY . .

# 生产构建
RUN cargo build --release

# ============================================
# 阶段二：运行镜像（极简尺寸）
# ============================================
FROM debian:bookworm-slim AS runtime

# 安装 CA 证书和 PostgreSQL 客户端库（运行时依赖）
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd -r app && useradd -r -g app -d /app -s /sbin/nologin app

WORKDIR /app

# 从构建阶段复制编译产物
COPY --from=builder /app/target/release/axum-api /app/axum-api
COPY --from=builder /app/.env.example /app/.env
COPY --from=builder /app/migrations /app/migrations

# 设置权限
RUN chown -R app:app /app

# 切换到非 root 用户
USER app

# 暴露端口
EXPOSE 8080

# 运行
CMD ["/app/axum-api"]
