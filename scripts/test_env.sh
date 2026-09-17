#!/usr/bin/env bash
# ============================================
# 本地集成测试依赖环境（PostgreSQL + Redis）
# ============================================
# 用真实的 Postgres/Redis 进程运行集成测试，无需 Docker。
#
#   scripts/test_env.sh start   # 启动并创建测试库
#   scripts/test_env.sh env     # 打印集成测试所需环境变量
#   scripts/test_env.sh stop    # 停止并清理
#
# 需要本机已安装 initdb / pg_ctl / redis-server（brew install postgresql redis）。
# ============================================
set -euo pipefail

PG_PORT="${PG_PORT:-55432}"
REDIS_PORT="${REDIS_PORT:-56379}"
PGDATA="${PGDATA:-/tmp/axum-api-pgtest}"
DB_NAME="${DB_NAME:-axum_api_test}"
PGUSER="${PGUSER:-postgres}"

TEST_DATABASE_URL="postgres://${PGUSER}@127.0.0.1:${PG_PORT}/${DB_NAME}"
TEST_REDIS_URL="redis://127.0.0.1:${REDIS_PORT}"

cmd_start() {
  if [ ! -d "${PGDATA}/base" ]; then
    echo "→ 初始化临时 Postgres 集群: ${PGDATA}"
    rm -rf "${PGDATA}"
    initdb -D "${PGDATA}" -U "${PGUSER}" -A trust --encoding=UTF8 >/dev/null
  fi

  if ! pg_ctl -D "${PGDATA}" status >/dev/null 2>&1; then
    echo "→ 启动 Postgres (端口 ${PG_PORT})"
    pg_ctl -D "${PGDATA}" -o "-p ${PG_PORT} -k /tmp -c listen_addresses=127.0.0.1" \
      -l "${PGDATA}/server.log" start >/dev/null
  fi

  if ! psql -h 127.0.0.1 -p "${PG_PORT}" -U "${PGUSER}" -lqt 2>/dev/null | cut -d'|' -f1 | grep -qw "${DB_NAME}"; then
    echo "→ 创建测试库 ${DB_NAME}"
    createdb -h 127.0.0.1 -p "${PG_PORT}" -U "${PGUSER}" "${DB_NAME}"
  fi

  if ! redis-cli -p "${REDIS_PORT}" ping >/dev/null 2>&1; then
    echo "→ 启动 Redis (端口 ${REDIS_PORT})"
    redis-server --port "${REDIS_PORT}" --daemonize yes --dir /tmp --save '' >/dev/null
  fi

  echo "✅ 测试依赖已就绪"
  cmd_env
}

cmd_stop() {
  pg_ctl -D "${PGDATA}" stop -m fast >/dev/null 2>&1 || true
  redis-cli -p "${REDIS_PORT}" shutdown nosave >/dev/null 2>&1 || true
  rm -rf "${PGDATA}"
  echo "✅ 已停止并清理"
}

cmd_env() {
  echo "export TEST_DATABASE_URL=\"${TEST_DATABASE_URL}\""
  echo "export TEST_REDIS_URL=\"${TEST_REDIS_URL}\""
  echo "export TEST_JWT_SECRET=\"integration-test-secret-value-0123456789\""
}

case "${1:-}" in
  start) cmd_start ;;
  stop) cmd_stop ;;
  env) cmd_env ;;
  *) echo "用法: $0 {start|stop|env}"; exit 1 ;;
esac
