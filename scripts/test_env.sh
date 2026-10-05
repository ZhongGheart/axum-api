#!/usr/bin/env bash
# ============================================
# 本地集成测试依赖环境（PostgreSQL + Redis + S3 mock）
# ============================================
# 用真实的 Postgres/Redis 进程运行集成测试，无需 Docker。
#
#   scripts/test_env.sh start   # 启动并创建测试库
#   scripts/test_env.sh env     # 打印集成测试所需环境变量
#   scripts/test_env.sh stop    # 停止并清理
#
# 需要本机已安装 initdb / pg_ctl / redis-server（brew install postgresql redis），
# 以及 pip install "moto[server]"（S3 mock，v0.27.0 起用于对象存储后端的集成测试）。
# ============================================
set -euo pipefail

PG_PORT="${PG_PORT:-55432}"
REDIS_PORT="${REDIS_PORT:-56379}"
MOTO_PORT="${MOTO_PORT:-59000}"
MOTO_BUCKET="${MOTO_BUCKET:-axum-test}"
MOTO_HOST="127.0.0.1"
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

  cmd_start_moto
  echo "✅ 测试依赖已就绪"
  cmd_env
}

# S3 mock（v0.27.0）
#
# 用 moto 而不是 minio 容器：本机没有 docker daemon，而 minio 的官方
# 下载地址现已全部 410。moto 是 S3 协议的 wire-level mock，
# 对 opendal 来说与真 S3 无差别——签名、状态码、404 语义都照协议来。
cmd_start_moto() {
  if ! command -v moto_server >/dev/null 2>&1; then
    echo "❌ 找不到 moto_server。S3 后端的集成测试需要它："
    echo "     pip3 install 'moto[server]'"
    exit 1
  fi
  if ! curl -s -o /dev/null "http://${MOTO_HOST}:${MOTO_PORT}/"; then
    echo "→ 启动 moto_server (端口 ${MOTO_PORT})"
    # 必须脱离当前 shell 的进程组：脚本 `set -e` 退出时
    # 同一进程组里的 moto 会被一起带走，下次 start 就得换个端口。
    # macOS 没有 setsid，用 python 的 start_new_session 达到同样效果。
    python3 - "$MOTO_PORT" <<'PYEOF' >/dev/null 2>&1
import subprocess, sys
port = sys.argv[1]
subprocess.Popen(
    ["moto_server", "-p", port, "-H", "127.0.0.1"],
    start_new_session=True,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
PYEOF
    for _ in $(seq 1 40); do
      curl -s -o /dev/null "http://${MOTO_HOST}:${MOTO_PORT}/" && break
      sleep 0.25
    done
  fi
  # bucket 必须存在：opendal 不会替你建，建 bucket 是部署方的责任，
  # 所以测试自己建——生产上忘了建的表现是第一次上传头像报 500。
  if ! curl -s -o /dev/null -w '%{http_code}' "http://${MOTO_HOST}:${MOTO_PORT}/${MOTO_BUCKET}" \
       | grep -q '^\(200\|404\)$'; then
    echo "❌ moto_server 起来了但不可达" >&2
    exit 1
  fi
  curl -s -o /dev/null -X PUT "http://${MOTO_HOST}:${MOTO_PORT}/${MOTO_BUCKET}"
  echo "→ moto bucket 就绪: ${MOTO_BUCKET}"
}

cmd_stop() {
  pg_ctl -D "${PGDATA}" stop -m fast >/dev/null 2>&1 || true
  redis-cli -p "${REDIS_PORT}" shutdown nosave >/dev/null 2>&1 || true
  pkill -f "moto_server -p ${MOTO_PORT}" >/dev/null 2>&1 || true
  rm -rf "${PGDATA}"
  echo "✅ 已停止并清理"
}

cmd_env() {
  echo "export TEST_DATABASE_URL=\"${TEST_DATABASE_URL}\""
  echo "export TEST_REDIS_URL=\"${TEST_REDIS_URL}\""
  echo "export TEST_JWT_SECRET=\"integration-test-secret-value-0123456789\""
  # S3 mock（v0.27.0）。凭据是假的：moto 不校验签名，只要求字段存在。
  echo "export TEST_S3_ENDPOINT=\"http://${MOTO_HOST}:${MOTO_PORT}\""
  echo "export TEST_S3_BUCKET=\"${MOTO_BUCKET}\""
  echo "export TEST_S3_ACCESS_KEY_ID=\"testkey\""
  echo "export TEST_S3_SECRET_ACCESS_KEY=\"testsecret\""
}

case "${1:-}" in
  start) cmd_start ;;
  stop) cmd_stop ;;
  env) cmd_env ;;
  *) echo "用法: $0 {start|stop|env}"; exit 1 ;;
esac
