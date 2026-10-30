# qe-device-pki

设备身份注册与 mTLS 通讯的服务端。协议契约见 [`docs/device-pki-protocol.md`](../../docs/device-pki-protocol.md)。

## 特性

- **一次性 token → 持久证书**：`auto` 型即时签发；`approval` 型由管理员审批后一次性下载
- **持有证明（PoP）**：客户端用 CSR 私钥签 `canonical_transcript`，防止 token 被截获后换别人的 CSR
- **挑战值**：60s TTL、绑定 `(token_hash, slot)`、一次性消费，抵抗重放
- **双 listener**：注册端点单向 TLS；业务端点 mTLS 且**每次握手查吊销状态**
- **防抖会话层**：seq 去重 + 250ms 窗口合并 + 心跳 5s×3 判定 offline
- **RFC 5280 CRL**：从 DB 吊销记录生成

## 环境变量

| 变量 | 说明 |
|---|---|
| `DATABASE_URL` | PostgreSQL 连接串 |
| `ENROLL_ADDR` | 注册 listener 地址，默认 `0.0.0.0:4440` |
| `MTLS_ADDR` | 业务 listener 地址，默认 `0.0.0.0:4441` |
| `ENROLL_TLS_CERT` / `ENROLL_TLS_KEY` | 注册端点证书/私钥（公开 CA 或自签） |
| `MTLS_TLS_CERT` / `MTLS_TLS_KEY` | 业务端点证书/私钥 |
| `CA_KEY` / `CA_CERT` | 内部 CA 签名密钥/证书，默认 `certs/ca-key.pem`、`certs/ca.pem` |
| `ADMIN_TOKEN` | 调用 `/revoke`、`/admin/...` 的 bearer token |

## 端点

注册 listener（单向 TLS）：

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/enroll/challenge` | 下发挑战值；approval 型同时创建 pending 单 |
| POST | `/enroll` | 提交 CSR + PoP；auto 直发证书，approval 返回 pending |
| GET | `/enroll/{id}` | approval 轮询；approved 后**一次性**下载 |
| POST | `/admin/enroll/{id}/decide` | 管理员批准/拒绝（Bearer） |
| GET | `/crl` | 签名 CRL |

业务 listener（mTLS，客户端证书必需）：

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/enroll/confirm` | 回连验证，`issued → active`，通讯打通 |
| POST | `/revoke` | 吊销（Bearer） |

## 关键实现说明

**吊销生效点**：`tls::RevocationCheckingVerifier` 在每次 mTLS 握手时查
`device_enrollment.revoked_at`。吊销后旧设备的下一次连接直接被拒，不依赖 CRL 分发。

**为什么两个 listener**：rustls 在握手阶段就要确定是否要求客户端证书，运行时切换会让状态机很难维护。

**CA 持久化**：`Ca::load(key_pem, cert_pem)` 从磁盘加载。`Ca::generate_self_signed` 仅供开发测试——生产环境重启后必须加载同一份 CA，否则已签发证书无法验证。

**注意**：`revoked` 检查在 rustls 的同步回调里做，用 `block_in_place`；单线程 runtime 下会降级放行，由 confirm/session 层补一道检查。若对吊销实时性要求极高，建议改为后台定期刷新吊销集合并走本地缓存。

## 测试

```bash
cargo test -p qe-device-pki      # 34 passed
```
