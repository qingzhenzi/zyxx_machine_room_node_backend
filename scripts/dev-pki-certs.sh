#!/usr/bin/env bash
# 生成 device-pki 本地开发证书（自签 CA + 注册 listener / mTLS listener 各一张服务端证书）。
#
# 仅用于本地联调 / 演示，禁止用于生产。
# 产物默认写到 deploy/device-pki/certs/（docker compose 的 device-pki 服务只读挂载）。
#
# 用法：
#   bash scripts/dev-pki-certs.sh                # 默认输出到 deploy/device-pki/certs
#   bash scripts/dev-pki-certs.sh /tmp/pki-certs # 指定输出目录
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/deploy/device-pki/certs}"
cd "$ROOT"

mkdir -p "$OUT"
# 容器内以非 root 用户（uid 10001）读取挂载证书：开发证书需可读；私钥仍是开发用，勿用于生产。
cargo run -q -p qe-device-pki --example gen_test_ca -- "$OUT"
chmod 0644 "$OUT"/*.pem 2>/dev/null || true

echo ""
echo "✓ device-pki 开发证书已生成：$OUT"
echo "  启动服务：docker compose --profile pki up -d --build device-pki"
echo "  或本地直接跑：DATABASE_URL=<url> \\"
echo "    ENROLL_ADDR=127.0.0.1:4440 MTLS_ADDR=127.0.0.1:4441 \\"
echo "    ENROLL_TLS_CERT=$OUT/enroll.pem ENROLL_TLS_KEY=$OUT/enroll-key.pem \\"
echo "    MTLS_TLS_CERT=$OUT/mtls.pem MTLS_TLS_KEY=$OUT/mtls-key.pem \\"
echo "    CA_KEY=$OUT/ca-key.pem CA_CERT=$OUT/ca.pem \\"
echo "    ADMIN_TOKEN=<管理端 token> ./target/debug/qe-device-pki"
