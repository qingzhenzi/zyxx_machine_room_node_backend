# qe-device-pki-client

设备 PKI 客户端：一次性注册、TOFU/pin 信任、mTLS 防抖通讯。

> **临时验证用 crate**。注册与通讯逻辑验证通过后，可原样移植进真实客户端项目。

## CLI

```bash
# 1. 用注册链接注册（auto 型直接拿证书）
qe-device-pki-client enroll "https://node:4440/enroll?token=T&slot=S" --trust

# approval 型：等待管理员批准（默认 5 分钟）
qe-device-pki-client enroll "https://...&token=T&slot=S" --trust --wait

# 2. mTLS 回连确认 —— 这一步成功才算通讯打通
qe-device-pki-client confirm

# 3. 心跳 + 防抖重连演示
qe-device-pki-client ping --count 5

# 4. 查看本地凭证
qe-device-pki-client status
```

`--data-dir` 可覆盖数据目录（默认 `~/.local/share/qe-device-pki-client`）。

## 服务端身份校验（三优先级）

1. **pin**：`...&pin=sha256/<hex>` 或 `pin=sha256spki/<hex>`，只接受该指纹
2. **已信任记录**：本地 trust store 里确认过的指纹
3. **TOFU**：首次连接展示指纹，`--trust` 表示人工确认，写入 trust store

无论哪条路径，客户端都会把「实际看到的证书指纹」作为 `tls_cert_seen_fp` 上报，
服务端据此做中间人检测（不一致返回 `fp_mismatch`）。

## 本地凭证

```
<data-dir>/
├── creds/<slot>/
│   ├── cert.pem        # 0600
│   ├── key.pem         # 0600
│   ├── ca-chain.pem    # 0600
│   └── meta.json       # 0600
├── trust-store.json
└── outbox/             # sled：未 ACK 消息队列
```

## 防抖通讯

- **指数退避**：100ms → 30s 上限，jitter 20%
- **消息可靠性**：出站消息单调 `seq`，写 sled 后才发送，收到 ACK 才删除；重连后按 seq 顺序重放
- **心跳**：5s

## 测试

```bash
cargo test -p qe-device-pki-client   # 33 passed
```
