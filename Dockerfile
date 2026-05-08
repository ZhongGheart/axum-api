# ============================================
# Dockerfile — 后端多阶段构建
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

WORKDIR /app

# ── 依赖缓存层 ──────────────────────────────
# 先复制 Cargo.toml 和 Cargo.lock（如果存在）
COPY Cargo.toml Cargo.lock* ./

# 创建虚拟 main.rs 以缓存依赖编译
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    mkdir migrations && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

# ── 完整编译 ─────────────────────────────────
COPY . .
RUN cargo build --release && \
    strip target/release/axum-api && \
    rm -rf target/release/build target/release/.fingerprint target/release/deps target/release/incremental

# ============================================
# 阶段二：运行镜像（极简 debian slim）
# ============================================
FROM debian:bookworm-slim AS runtime

# 安装运行时依赖
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd -r app && useradd -r -g app -d /app -s /sbin/nologin app

WORKDIR /app

# 复制编译产物和运行时文件
COPY --from=builder /app/target/release/axum-api /app/axum-api
COPY --from=builder /app/.env.example /app/.env
COPY --from=builder /app/migrations /app/migrations

# 安全配置
RUN chown -R app:app /app && \
    chmod 500 /app/axum-api && \
    chmod 400 /app/.env

USER app

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/app/axum-api"]

EXPOSE 8080

CMD ["/app/axum-api"]
