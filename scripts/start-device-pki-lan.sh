#!/usr/bin/env bash
# 开发/演示：以「LAN 可访问」方式前台启动 device-pki（0.0.0.0:4440/4441）。
#
# 背景：compose profile `pki` 默认只在本机可用；远程机（如 192.168.8.141）上的
# qe-agent 需要能直接访问控制节点的 4440/4441，且镜像分片的预签名 URL 必须指向
# 远程机可达的 RustFS 地址（不能是 127.0.0.1）。本脚本把这两件事一次做对：
#   - 证书 SAN 检查（必须含控制节点 LAN IP，否则远程 agent TLS 握手失败）；
#   - 绑定 0.0.0.0:4440/4441 + PUBLIC_* 用 LAN 地址；
#   - S3_ENDPOINT 用 LAN 地址（presign 出来的分片 URL 远程机可下载）。
#
# 用法（在 room_node 仓库根目录）：
#   bash scripts/start-device-pki-lan.sh                 # 自动探测 LAN IP
#   LAN_IP=192.168.8.231 bash scripts/start-device-pki-lan.sh
#   nohup setsid bash scripts/start-device-pki-lan.sh > /tmp/qe-device-pki.log 2>&1 &
#
# 前置：
#   1) 证书已生成且含 LAN IP SAN：
#        DEVICE_PKI_EXTRA_SANS="192.168.8.231" bash scripts/dev-pki-certs.sh
#      （换 CA 会导致已注册设备全部失效；已注册设备可参考 docs/lan-pki-setup.md 附录 A
#       保留原 CA 只补签服务端证书）
#   2) .env 里 S3_* / STORAGE_SIGNING_KEY_BASE64 与 qe-api 一致；
#   3) RustFS 的 9000 端口对 LAN 可达（compose.yaml 已改为 0.0.0.0:9000）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CERT_DIR="${DEVICE_PKI_CERT_DIR:-$ROOT/deploy/device-pki/certs}"
BIN="${DEVICE_PKI_BIN:-$ROOT/target/debug/qe-device-pki}"

die() { printf '[start-device-pki-lan] 错误：%s\n' "$*" >&2; exit 1; }
log() { printf '[start-device-pki-lan] %s\n' "$*"; }

[[ -f "$BIN" ]] || die "找不到 $BIN；先构建：cargo build -p qe-device-pki"
[[ -f "$CERT_DIR/ca.pem" ]] || die "缺少证书 $CERT_DIR/ca.pem；先跑 scripts/dev-pki-certs.sh"
[[ -f .env ]] || die "缺少 $ROOT/.env（需要 DATABASE_URL / S3_* / STORAGE_SIGNING_KEY_BASE64）"

# .env 只用于补齐缺失变量；已显式 export 的不覆盖。
set -a
# shellcheck disable=SC1091
. ./.env
set +a

# 自动探测默认路由的源地址（LAN IP）。
if [[ -z "${LAN_IP:-}" ]]; then
    LAN_IP="$(ip route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}')"
fi
[[ -n "${LAN_IP:-}" ]] || die "无法探测 LAN IP，请显式设置 LAN_IP=<控制节点局域网地址>"
log "控制节点 LAN IP：$LAN_IP"

# 证书 SAN 必须覆盖 LAN IP，否则远程 agent 校验失败（certificate not valid for name）。
for cert in enroll.pem mtls.pem; do
    if ! openssl x509 -in "$CERT_DIR/$cert" -noout -ext subjectAltName 2>/dev/null | grep -q "IP Address:$LAN_IP"; then
        die "$CERT_DIR/$cert 的 SAN 不含 IP:$LAN_IP（远程机 TLS 会失败）。
      修复：DEVICE_PKI_EXTRA_SANS=\"$LAN_IP\" bash scripts/dev-pki-certs.sh
      或按 docs/lan-pki-setup.md 附录 A 保留原 CA 只补签服务端证书。"
    fi
done
log "证书 SAN 校验通过（含 IP:$LAN_IP）"

# 端口占用检查：避免和已有 device-pki 实例打架。
for port in 4440 4441; do
    if ss -ltn 2>/dev/null | grep -q ":$port "; then
        die "端口 $port 已被占用（可能已有 device-pki 在跑）。
      先停：pkill -x qe-device-pki（或 docker compose --profile pki stop device-pki）"
    fi
done

export ENROLL_ADDR="${ENROLL_ADDR:-0.0.0.0:4440}"
export MTLS_ADDR="${MTLS_ADDR:-0.0.0.0:4441}"
export PUBLIC_ENROLL_ENDPOINT="${PUBLIC_ENROLL_ENDPOINT:-https://$LAN_IP:4440}"
export PUBLIC_MTLS_ENDPOINT="${PUBLIC_MTLS_ENDPOINT:-https://$LAN_IP:4441}"
export ENROLL_TLS_CERT="$CERT_DIR/enroll.pem"
export ENROLL_TLS_KEY="$CERT_DIR/enroll-key.pem"
export MTLS_TLS_CERT="$CERT_DIR/mtls.pem"
export MTLS_TLS_KEY="$CERT_DIR/mtls-key.pem"
export CA_KEY="$CERT_DIR/ca-key.pem"
export CA_CERT="$CERT_DIR/ca.pem"
export ADMIN_TOKEN="${ADMIN_TOKEN:-${DEVICE_PKI_ADMIN_TOKEN:-dev-admin-token-change-me}}"
export ADMIN_RATE_LIMIT="${ADMIN_RATE_LIMIT:-30}"
export INTERNAL_ADDR="${INTERNAL_ADDR:-127.0.0.1:4442}"
export INTERNAL_TOKEN="${INTERNAL_TOKEN:-${DEVICE_PKI_INTERNAL_TOKEN:-dev-internal-token-change-me}}"
# 预签名 URL 的 host 必须远程机可达：默认用 LAN IP，不要用 127.0.0.1。
export S3_ENDPOINT="${DEVICE_PKI_S3_ENDPOINT:-http://$LAN_IP:${S3_PORT:-9000}}"
export STORAGE_PRESIGN_TTL_SECONDS="${STORAGE_PRESIGN_TTL_SECONDS:-900}"
# device-pki 不读 .env 的 RUST_LOG 以外的约定变量；这里给默认值。
export RUST_LOG="${RUST_LOG:-info,qe_device_pki=debug}"

log "监听：ENROLL_ADDR=$ENROLL_ADDR MTLS_ADDR=$MTLS_ADDR"
log "对外：PUBLIC_ENROLL_ENDPOINT=$PUBLIC_ENROLL_ENDPOINT PUBLIC_MTLS_ENDPOINT=$PUBLIC_MTLS_ENDPOINT"
log "S3 预签名：S3_ENDPOINT=$S3_ENDPOINT"
log "前台启动 $BIN（Ctrl-C 退出；演示可用 nohup setsid 后台化）"
exec "$BIN"
