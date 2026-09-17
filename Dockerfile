# ============================================
# Dockerfile — 后端多阶段构建
# ============================================
# 阶段一：编译构建
#
# 注意：镜像版本必须与 Cargo.lock 的实际要求一致。
# Cargo.lock 中部分依赖使用 edition2024，Rust 1.82 无法解析（构建会直接失败），
# 因此这里固定在 1.93；CI 的 docker job 会持续校验该镜像可构建。
# ============================================
FROM rust:1.93-slim-bookworm AS builder

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
    curl \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd -r app && useradd -r -g app -d /app -s /sbin/nologin app

WORKDIR /app

# 复制编译产物（迁移脚本已由 sqlx::migrate! 嵌入二进制）
# 运行配置一律通过环境变量注入，不把 .env 打进镜像
COPY --from=builder /app/target/release/axum-api /app/axum-api

# 安全配置
RUN chown -R app:app /app && chmod 500 /app/axum-api

USER app

# 探测真实健康端点（任一依赖不可用时返回 503）
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1

EXPOSE 8080

CMD ["/app/axum-api"]
