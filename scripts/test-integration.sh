#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# compose.yaml 对 JWT_SECRET 用了必填插值；集成测试不需要启动 API，
# 这里给一个仅用于 compose 解析的占位值（API 进程不会使用它）。
export JWT_SECRET="${JWT_SECRET:-integration-only-secret-0123456789abcdef}"

echo "Starting PostgreSQL and Redis..."
docker compose up -d postgres redis

echo "Waiting for PostgreSQL..."
for attempt in $(seq 1 60); do
  if docker compose exec -T postgres pg_isready -U qestudio -d qestudio >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

echo "Ensuring qestudio_test database exists..."
docker compose exec -T postgres psql -U qestudio -d qestudio -tc \
  "SELECT 1 FROM pg_database WHERE datname = 'qestudio_test'" \
  | grep -q 1 || docker compose exec -T postgres createdb -U qestudio qestudio_test

echo "Waiting for Redis..."
for attempt in $(seq 1 60); do
  if docker compose exec -T redis redis-cli ping >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

export TEST_DATABASE_URL="${TEST_DATABASE_URL:-postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test}"
export TEST_REDIS_URL="${TEST_REDIS_URL:-redis://127.0.0.1:6379}"

echo "Running integration tests against $TEST_DATABASE_URL"
cargo test -p qe-api --test integration -- --ignored --nocapture

echo "Integration tests passed."
echo "Services remain running; stop with: docker compose stop postgres redis"
