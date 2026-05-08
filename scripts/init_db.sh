#!/usr/bin/env bash
# ============================================
# 数据库初始化脚本
# 用于首次部署时创建数据库并运行迁移
# ============================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ── 配置 ──────────────────────────────────────────────────────
DB_NAME="${DB_NAME:-axum_api}"
DB_USER="${DB_USER:-postgres}"
DB_PASS="${DB_PASSWORD:-password}"
DB_HOST="${DB_HOST:-localhost}"
DB_PORT="${DB_PORT:-5432}"

echo "============================================"
echo " Axum API — 数据库初始化"
echo "============================================"
echo "数据库:   ${DB_NAME}"
echo "主机:     ${DB_HOST}:${DB_PORT}"
echo "用户:     ${DB_USER}"
echo ""

# ── 检查 psql 是否可用 ──────────────────────────────────────
if ! command -v psql &>/dev/null; then
  echo "❌ 错误: 找不到 psql 命令"
  echo "   请安装 PostgreSQL 客户端:"
  echo "   brew install postgresql"
  echo "   # 或使用 Docker:"
  echo "   docker run -d --name postgres \\"
  echo "     -e POSTGRES_PASSWORD=${DB_PASS} \\"
  echo "     -p ${DB_PORT}:5432 \\"
  echo "     postgres:16-alpine"
  exit 1
fi

# ── 连接 URL（用于迁移） ────────────────────────────────────
export DATABASE_URL="postgres://${DB_USER}:${DB_PASS}@${DB_HOST}:${DB_PORT}/${DB_NAME}"

# ── 创建数据库（如果不存在） ─────────────────────────────────
echo "→ 创建数据库 ${DB_NAME}..."
psql -U "${DB_USER}" -h "${DB_HOST}" -p "${DB_PORT}" -tc \
  "SELECT 1 FROM pg_database WHERE datname = '${DB_NAME}'" \
  | grep -q 1 \
  || psql -U "${DB_USER}" -h "${DB_HOST}" -p "${DB_PORT}" -c \
    "CREATE DATABASE ${DB_NAME}"

echo "✓ 数据库就绪"

# ── 运行迁移 ──────────────────────────────────────────────────
echo ""
echo "→ 运行数据库迁移..."
if command -v sqlx &>/dev/null; then
  sqlx migrate run
  echo "✓ 迁移完成"
else
  echo "⚠  未安装 sqlx-cli，直接执行 SQL 文件..."
  for f in migrations/*.sql; do
    echo "   执行 $f ..."
    psql -U "${DB_USER}" -h "${DB_HOST}" -p "${DB_PORT}" -d "${DB_NAME}" -f "$f"
  done
  echo "✓ SQL 执行完成"
fi

# ── 验证 ──────────────────────────────────────────────────────
echo ""
echo "→ 验证数据库表..."
psql -U "${DB_USER}" -h "${DB_HOST}" -p "${DB_PORT}" -d "${DB_NAME}" -c \
  "\dt"

echo ""
echo "============================================"
echo " ✅ 数据库初始化完成"
echo "============================================"
echo ""
echo " DATABASE_URL=${DATABASE_URL}"
echo ""
echo " 启动 API 服务:"
echo "   cargo run"
