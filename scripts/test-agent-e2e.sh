#!/usr/bin/env bash
# qe-agent 端到端回归：device-pki（真实 mTLS）+ enroll + 遥测上报 + 本机 IPC。
#
# 前提：本机 PostgreSQL 可连（默认 qestudio_test），curl/psql/python3 可用；
#       qestudio_test 需已迁移到最新（脚本会在 device-pki 启动后校验，未就绪时给出明确错误）。
# 用法：
#   bash scripts/test-agent-e2e.sh
#   TEST_DATABASE_URL=... bash scripts/test-agent-e2e.sh
#
# 新版统一 agent（room_agent HEAD）CLI：
#   - 注册：`qe-agent --data-dir <dir> cli enroll <link> --trust`
#   - 遥测：`qe-agent --data-dir <dir> serve --role machine`（常驻；没有 --once 选项）
#     → 本脚本后台启动 serve，收到 full 快照后立即结束进程，等价于单轮上报，
#       顺带覆盖 IPC（--ipc-socket）与 remote policy 拉取。
#
# 全程使用 target/ 下的临时目录与临时端口（14440/14441），结束时清理测试数据与进程。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# room_node 的父目录下是 room_agent（2026-11-02 目录重组后的布局）
AGENT_ROOT="$(cd "$ROOT/../room_agent" && pwd)"
cd "$ROOT"

DB_URL="${TEST_DATABASE_URL:-postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test}"
ENROLL_PORT=14440
MTLS_PORT=14441
ADMIN_TOKEN="e2e-agent-admin-$(date +%s)"
mkdir -p "$ROOT/target"
TMP_DIR="$(mktemp -d "$ROOT/target/qe-agent-e2e.XXXXXX")"
SLOT="e2e-agent-$(date +%s)"
DEVICE_PKI_PID=""
AGENT_PID=""
SERIAL=""
KEEP_TMP=0

log() { printf '\033[36m[agent-e2e]\033[0m %s\n' "$*"; }
fail() {
    printf '\033[31m[agent-e2e] FAIL: %s\033[0m\n' "$*" >&2
    KEEP_TMP=1
    exit 1
}

cleanup() {
    set +e
    [[ -n "${AGENT_PID}" ]] && kill "${AGENT_PID}" 2>/dev/null && wait "${AGENT_PID}" 2>/dev/null
    [[ -n "${DEVICE_PKI_PID}" ]] && kill "${DEVICE_PKI_PID}" 2>/dev/null && wait "${DEVICE_PKI_PID}" 2>/dev/null
    # 清理测试数据（顺序受外键约束影响；0027/0028 后 device_id 为可空列，不参与外键）
    if command -v psql >/dev/null 2>&1; then
        psql "$DB_URL" -q -c "DELETE FROM machine_snapshots WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_session WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_downlink WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_message_inbox WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_enroll_challenge WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_bootstrap_token WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_enrollment WHERE device_slot='${SLOT}'" >/dev/null 2>&1
    fi
    if [[ "${KEEP_TMP}" == "0" ]]; then
        rm -rf "${TMP_DIR}"
    else
        printf '\033[33m[agent-e2e] 现场保留: %s\033[0m\n' "${TMP_DIR}" >&2
    fi
}
trap cleanup EXIT

command -v psql >/dev/null 2>&1 || fail "缺少 psql"
command -v curl >/dev/null 2>&1 || fail "缺少 curl"
command -v python3 >/dev/null 2>&1 || fail "缺少 python3"
[[ -x "$AGENT_ROOT/target/debug/qe-agent" ]] || fail "缺少 $AGENT_ROOT/target/debug/qe-agent（先 cargo build --manifest-path $AGENT_ROOT/Cargo.toml -p qe-agent）"
psql "$DB_URL" -tAc 'select 1' >/dev/null 2>&1 \
    || fail "连不上 PostgreSQL: $DB_URL（可先 scripts/test-integration.sh 准备测试库）"

# 迁移前置检查：qestudio_test 必须已迁移；device-pki 启动时也会自动 migrate（失败会写日志）。
MIG_BEFORE="$(psql "$DB_URL" -tAc "select count(*) from _sqlx_migrations" 2>/dev/null || true)"
if [[ -z "${MIG_BEFORE}" || "${MIG_BEFORE}" -lt 28 ]]; then
    log "警告：_sqlx_migrations=${MIG_BEFORE:-0}/28，device-pki 启动时将自动迁移；若失败请检查迁移日志"
fi

log "构建 qe-device-pki / qe-agent"
cargo build -q -p qe-device-pki
cargo build -q --manifest-path "$AGENT_ROOT/Cargo.toml" -p qe-agent

log "生成临时 CA 与服务端证书"
cargo run -q -p qe-device-pki --example gen_test_ca -- "$TMP_DIR/certs" >/dev/null

log "启动 device-pki（enroll :$ENROLL_PORT / mTLS :$MTLS_PORT，DB=$DB_URL）"
DATABASE_URL="$DB_URL" \
ENROLL_ADDR="127.0.0.1:$ENROLL_PORT" MTLS_ADDR="127.0.0.1:$MTLS_PORT" \
ENROLL_TLS_CERT="$TMP_DIR/certs/enroll.pem" ENROLL_TLS_KEY="$TMP_DIR/certs/enroll-key.pem" \
MTLS_TLS_CERT="$TMP_DIR/certs/mtls.pem" MTLS_TLS_KEY="$TMP_DIR/certs/mtls-key.pem" \
CA_KEY="$TMP_DIR/certs/ca-key.pem" CA_CERT="$TMP_DIR/certs/ca.pem" \
ADMIN_TOKEN="$ADMIN_TOKEN" \
PUBLIC_ENROLL_ENDPOINT="https://localhost:$ENROLL_PORT" \
PUBLIC_MTLS_ENDPOINT="https://localhost:$MTLS_PORT" \
RUST_LOG=warn ./target/debug/qe-device-pki >"$TMP_DIR/device-pki.log" 2>&1 &
DEVICE_PKI_PID=$!

for _ in $(seq 1 60); do
    if curl -sk -o /dev/null -w '%{http_code}' "https://localhost:$ENROLL_PORT/crl" 2>/dev/null | grep -q 200; then
        break
    fi
    sleep 0.25
done
curl -sk -o /dev/null "https://localhost:$ENROLL_PORT/crl" \
    || fail "device-pki 未就绪（见 $TMP_DIR/device-pki.log）"

MIG_AFTER="$(psql "$DB_URL" -tAc "select count(*) from _sqlx_migrations" 2>/dev/null || echo 0)"
[[ "${MIG_AFTER:-0}" -ge 28 ]] \
    || fail "测试库迁移不完整（_sqlx_migrations=${MIG_AFTER:-0}/28），见 $TMP_DIR/device-pki.log"
log "OK  PostgreSQL 迁移就绪（${MIG_AFTER}/28）"

log "签发一次性 token 并 enroll（slot=$SLOT）"
TOKEN_JSON="$(curl -sk -X POST "https://localhost:$ENROLL_PORT/admin/tokens" \
    -H "Authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
    -d "{\"kind\":\"auto\",\"slot\":\"$SLOT\",\"ttl_secs\":600}")"
LINK="$(printf '%s' "$TOKEN_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["link"])')"
DATA_DIR="$TMP_DIR/agent-data"
"$AGENT_ROOT/target/debug/qe-agent" --data-dir "$DATA_DIR" cli enroll "$LINK" --trust \
    >"$TMP_DIR/enroll.log" 2>&1 || fail "enroll 失败（见 $TMP_DIR/enroll.log）"
SERIAL="$(python3 - "$DATA_DIR" <<'PY'
import json, pathlib, sys
creds = pathlib.Path(sys.argv[1]) / "creds"
meta = json.loads((creds / next(creds.iterdir()).name / "meta.json").read_text())
print(meta["serial"])
PY
)"
log "OK  enrolled serial=$SERIAL"

log "等待 device-pki 证书状态缓存刷新（5s 窗口）"
sleep 6

# serve 常驻模式会跑 boot 流水线（防火墙/展示层/P2P/检查）：
# 用 agent-config.json 关掉防火墙与展示层，避免测试机被改配置；P2P 线程随进程退出。
mkdir -p "$DATA_DIR"
printf '{"boot":{"firewall_enabled":false,"display":{"enabled":false}}}\n' >"$DATA_DIR/agent-config.json"

log "mTLS 遥测：serve --role machine（收到 full 快照后结束进程；新版 CLI 无 --once）"
IPC_SOCKET="$TMP_DIR/agent.sock"
DEVICE_PKI_MTLS_URL="https://localhost:$MTLS_PORT" DEVICE_SLOT="$SLOT" \
DEVICE_DATA_DIR="$DATA_DIR" REPORT_MODE=mtls \
ROOM_AGENT_DISABLE_HEALTH=true ROOM_AGENT_DISABLE_DIAGNOSE=true ROOM_AGENT_DISABLE_SECURITY=true \
ROOM_AGENT_DISABLE_DOCKER=true ROOM_AGENT_DISABLE_VMS=true \
RUST_LOG=warn "$AGENT_ROOT/target/debug/qe-agent" --data-dir "$DATA_DIR" \
    serve --role machine --report-mode mtls --ipc-socket "$IPC_SOCKET" \
    >"$TMP_DIR/agent.log" 2>&1 &
AGENT_PID=$!

ROWS=0
# serve 会先跑 boot 流水线（检查/探测）再进入遥测：机器负载高时可能 >30s，
# 给到 90s 上限，避免与其它构建并行时误报失败（实测 T2 并发构建时曾 30s 超时）。
for _ in $(seq 1 180); do
    ROWS="$(psql "$DB_URL" -tAc "select count(*) from machine_snapshots where serial='$SERIAL' and tier='full'" 2>/dev/null || echo 0)"
    [[ "${ROWS:-0}" -ge 1 ]] && break
    kill -0 "$AGENT_PID" 2>/dev/null || fail "qe-agent serve 提前退出（见 $TMP_DIR/agent.log）"
    sleep 0.5
done
[[ "${ROWS:-0}" -ge 1 ]] || fail "90s 内未收到 full 快照，实际 $ROWS 行（见 $TMP_DIR/agent.log）"
log "OK  machine_snapshots: serial=$SERIAL tier=full rows=$ROWS"

log "本机 IPC 冒烟（serve --ipc-socket）"
for _ in $(seq 1 40); do
    [[ -S "$IPC_SOCKET" ]] && break
    sleep 0.1
done
[[ -S "$IPC_SOCKET" ]] || fail "IPC socket 未就绪（见 $TMP_DIR/agent.log）"
STATUS="$(curl -s --unix-socket "$IPC_SOCKET" http://localhost/v1/status)"
printf '%s' "$STATUS" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["enrolled"] is True; assert d["serial"]; print("OK  IPC status serial=" + d["serial"])' \
    || fail "IPC status 异常: $STATUS"
curl -s --unix-socket "$IPC_SOCKET" 'http://localhost/v1/logs?limit=5' | python3 -c 'import json,sys; json.load(sys.stdin)' \
    || fail "IPC logs 异常"
log "OK  IPC logs"

# 收到快照与 IPC 证据后结束常驻进程（cleanup 兜底）。
kill "$AGENT_PID" 2>/dev/null || true
wait "$AGENT_PID" 2>/dev/null || true
AGENT_PID=""

log "PASS  qe-agent end-to-end（device-pki mTLS + enroll + telemetry report + IPC）"
