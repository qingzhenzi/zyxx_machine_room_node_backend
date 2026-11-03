#!/usr/bin/env bash
# qe-agent 端到端回归：device-pki（真实 mTLS）+ enroll + 遥测上报 + 本机 IPC。
#
# 前提：本机 PostgreSQL 可连（默认 qestudio_test），curl/psql/python3 可用。
# 用法：
#   bash scripts/test-agent-e2e.sh
#   TEST_DATABASE_URL=... bash scripts/test-agent-e2e.sh
#
# 全程使用临时目录与临时端口（14440/14441），结束时清理测试数据与进程。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT_ROOT="$(cd "$ROOT/../../room_agent" && pwd)"
cd "$ROOT"

DB_URL="${TEST_DATABASE_URL:-postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test}"
ENROLL_PORT=14440
MTLS_PORT=14441
ADMIN_TOKEN="e2e-agent-admin-$(date +%s)"
TMP_DIR="$(mktemp -d /tmp/qe-agent-e2e.XXXXXX)"
SLOT="e2e-agent-$(date +%s)"
DEVICE_PKI_PID=""
IPC_PID=""
SERIAL=""

log() { printf '\033[36m[agent-e2e]\033[0m %s\n' "$*"; }
fail() { printf '\033[31m[agent-e2e] FAIL: %s\033[0m\n' "$*" >&2; exit 1; }

cleanup() {
    set +e
    [[ -n "${IPC_PID}" ]] && kill "${IPC_PID}" 2>/dev/null
    [[ -n "${DEVICE_PKI_PID}" ]] && kill "${DEVICE_PKI_PID}" 2>/dev/null
    # 清理测试数据（顺序受外键约束影响）
    if command -v psql >/dev/null 2>&1; then
        psql "$DB_URL" -q -c "DELETE FROM machine_snapshots WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_session WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_downlink WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_message_inbox WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_enroll_challenge WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_bootstrap_token WHERE device_slot='${SLOT}'" >/dev/null 2>&1
        psql "$DB_URL" -q -c "DELETE FROM device_enrollment WHERE device_slot='${SLOT}'" >/dev/null 2>&1
    fi
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

command -v psql >/dev/null 2>&1 || fail "缺少 psql"
command -v curl >/dev/null 2>&1 || fail "缺少 curl"
command -v python3 >/dev/null 2>&1 || fail "缺少 python3"
psql "$DB_URL" -tAc 'select 1' >/dev/null 2>&1 || fail "连不上 PostgreSQL: $DB_URL（可先 scripts/test-integration.sh 准备）"

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

for _ in $(seq 1 40); do
    if curl -sk -o /dev/null -w '%{http_code}' "https://localhost:$ENROLL_PORT/crl" 2>/dev/null | grep -q 200; then
        break
    fi
    sleep 0.25
done
curl -sk -o /dev/null "https://localhost:$ENROLL_PORT/crl" || fail "device-pki 未就绪（见 $TMP_DIR/device-pki.log）"

log "签发一次性 token 并 enroll（slot=$SLOT）"
TOKEN_JSON="$(curl -sk -X POST "https://localhost:$ENROLL_PORT/admin/tokens" \
    -H "Authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
    -d "{\"kind\":\"auto\",\"slot\":\"$SLOT\",\"ttl_secs\":600}")"
LINK="$(printf '%s' "$TOKEN_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["link"])')"
DATA_DIR="$TMP_DIR/agent-data"
"$AGENT_ROOT/target/debug/qe-agent" --data-dir "$DATA_DIR" enroll "$LINK" --trust >"$TMP_DIR/enroll.log" 2>&1 || fail "enroll 失败（见 $TMP_DIR/enroll.log）"
SERIAL="$(python3 - "$DATA_DIR" <<'PY'
import json, pathlib, sys
meta = json.loads((pathlib.Path(sys.argv[1]) / "creds" / next((pathlib.Path(sys.argv[1]) / "creds").iterdir()).name / "meta.json").read_text())
print(meta["serial"])
PY
)"
log "enrolled serial=$SERIAL"

log "等待 device-pki 证书状态缓存刷新（5s 窗口）"
sleep 6

log "mTLS 上报一轮（--role machine --once）"
DEVICE_PKI_MTLS_URL="https://localhost:$MTLS_PORT" DEVICE_SLOT="$SLOT" DEVICE_DATA_DIR="$DATA_DIR" \
REPORT_MODE=mtls \
ROOM_AGENT_DISABLE_HEALTH=true ROOM_AGENT_DISABLE_DIAGNOSE=true ROOM_AGENT_DISABLE_SECURITY=true \
ROOM_AGENT_DISABLE_DOCKER=true ROOM_AGENT_DISABLE_VMS=true \
RUST_LOG=warn ./target/debug/qe-agent run --role machine --once 2>"$TMP_DIR/report.log" \
    || fail "mTLS 上报失败（见 $TMP_DIR/report.log）"

ROWS="$(psql "$DB_URL" -tAc "select count(*) from machine_snapshots where serial='$SERIAL' and tier='full'")"
[[ "$ROWS" == "1" ]] || fail "machine_snapshots 期望 1 行 full 快照，实际 $ROWS"
log "OK  machine_snapshots: serial=$SERIAL tier=full"

log "本机 IPC 冒烟（qe-agent serve）"
IPC_SOCKET="$TMP_DIR/agent.sock"
./target/debug/qe-agent --data-dir "$DATA_DIR" serve --ipc-socket "$IPC_SOCKET" >"$TMP_DIR/ipc.log" 2>&1 &
IPC_PID=$!
for _ in $(seq 1 40); do
    [[ -S "$IPC_SOCKET" ]] && break
    sleep 0.1
done
STATUS="$(curl -s --unix-socket "$IPC_SOCKET" http://localhost/v1/status)"
printf '%s' "$STATUS" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["enrolled"] is True; assert d["serial"]; print("OK  IPC status serial=" + d["serial"])' \
    || fail "IPC status 异常: $STATUS"
curl -s --unix-socket "$IPC_SOCKET" 'http://localhost/v1/logs?limit=5' | python3 -c 'import json,sys; json.load(sys.stdin)' \
    || fail "IPC logs 异常"
log "OK  IPC logs"

log "PASS  qe-agent end-to-end（enroll → mTLS report → IPC）"
