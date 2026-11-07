# device-pki 开发部署

`qe-device-pki` 是设备注册（一次性 token → CSR → 证书）与 mTLS 通讯服务端，
默认监听：

- 注册 listener（单向 TLS）：`4440`（`/enroll/*`、`/admin/*`、`/crl`）
- mTLS listener（业务）：`4441`（`/machine/report`、`/agent/config`、`/messages`、`/channel`、`/resources/presign`）

## 本地 compose（profile=pki）

```bash
# 1) 生成开发自签证书（写到 deploy/device-pki/certs/，已被 .gitignore 忽略）
bash scripts/dev-pki-certs.sh

# 2) 启动（复用 api 的镜像，单镜像内含 qe-api + qe-device-pki）
docker compose --profile pki up -d --build device-pki
docker compose --profile pki logs -f device-pki
```

前置条件：

- `deploy/device-pki/certs/` 下存在 `ca.pem`、`ca-key.pem`、`enroll.pem`、`enroll-key.pem`、
  `mtls.pem`、`mtls-key.pem`（由 `scripts/dev-pki-certs.sh` 生成，仅限开发）。
- `.env` 中 `DEVICE_PKI_ADMIN_TOKEN` 换成长随机值（`/admin/*` 静态 Bearer token）。
- 需要资源/镜像预签名时，`S3_*` 与 `STORAGE_SIGNING_KEY_BASE64` 必须与 qe-api 完全一致，
  否则 manifest 验签失败。

## 直接跑二进制（不用容器）

```bash
DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio \
ENROLL_ADDR=127.0.0.1:4440 MTLS_ADDR=127.0.0.1:4441 \
ENROLL_TLS_CERT=deploy/device-pki/certs/enroll.pem ENROLL_TLS_KEY=deploy/device-pki/certs/enroll-key.pem \
MTLS_TLS_CERT=deploy/device-pki/certs/mtls.pem MTLS_TLS_KEY=deploy/device-pki/certs/mtls-key.pem \
CA_KEY=deploy/device-pki/certs/ca-key.pem CA_CERT=deploy/device-pki/certs/ca.pem \
ADMIN_TOKEN=dev-admin-token-change-me \
cargo run -p qe-device-pki
```

> 设备注册与 mTLS 遥测契约见 `docs/device-pki-protocol.md`（v0.2）与
> `docs/machine-telemetry.md`（v1.2）。E2E 回归：`make agent-e2e`。
