# 设备注册与 mTLS 通讯协议契约（草案 v0.1）

## 1. 总览

两阶段通道建立：

1. **阶段 1（注册）**：单向 TLS（pin / TOFU / 系统 CA），客户端拉取信息并完成注册
2. **阶段 2（业务）**：mTLS 双向验证。`POST /enroll/confirm` 握手成功 ⇔ 通讯真正打通

注册成功 ≠ 通讯打通。`issued` → `active` 以首次 mTLS `confirm` 为准。

## 2. 角色

- **管理端**：创建节点（slot），生成注册链接，审批 approval 型注册单，吊销证书
- **服务端（qe-device-pki）**：CA、token/challenge 管理、注册、吊销、CRL、防抖会话状态
- **客户端（qe-device-pki-client，临时 crate）**：enroll CLI、凭证持久化、防抖 mTLS 通讯

## 3. Token 模型

两类 token，同表存储，`kind` 区分：

| kind | 语义 | 流程 |
|---|---|---|
| `auto` | 即时批准。服务端已备好节点信息 | challenge → enroll → 直接返回证书 |
| `approval` | 人工批准。客户端上报注册信息 | challenge → enroll(pending) → 轮询 → 批准后一次性下载 |

共性：一次性、短 TTL（默认 15 min）、库中只存 SHA-256 hash、使用后立即置 `used_at`。

注册链接格式：`{endpoint}?token=T&slot=S`，slot 显式绑定，先到先得，后到 409。无随机池。

## 4. 端点（注册 listener，单向 TLS）

### POST /enroll/challenge

请求：
```json
{ "token": "T", "slot": "S", "client_info": { "hostname": "...", "machine_id": "..." } }
```
- `client_info` 仅 approval 型必填
- 校验：token 有效未用未过期、slot 匹配；approval 型创建 pending 注册单
- 挑战绑定三元组 `(token_hash, slot, tls_cert_seen_fp_hint)`

响应：
```json
{ "challenge_id": "uuid", "challenge": "<hex 32B>", "ttl_secs": 60 }
```

### POST /enroll

请求：
```json
{
  "challenge_id": "uuid",
  "csr_der": "<base64>",
  "pop_signature": "<base64 ECDSA/Ed25519>",
  "tls_cert_seen_fp": "<hex sha256 of server cert seen by client>"
}
```
- `canonical_transcript = challenge || token_hash || slot || sha256(csr_der) || tls_cert_seen_fp`
- 服务端验证：challenge 未过期未消费且三元组匹配、PoP 签名、（TOFU 场景）指纹对比
- 事务：`UPDATE challenge SET consumed_at`、`UPDATE token SET used_at`、插 enrollment

auto 型响应：
```json
{ "status": "issued", "cert_pem": "...", "ca_chain_pem": "...", "serial": "...", "mtls_endpoint": "host:port" }
```

approval 型响应：
```json
{ "status": "pending", "enroll_id": "uuid", "poll_after_secs": 5 }
```

### GET /enroll/{id}  （approval 型轮询）

- `pending` / `rejected` / `approved`
- approved 时**一次性**返回证书（下载后 `downloaded_at` 置位，不可再取）

### POST /enroll/confirm  （mTLS listener，业务 CA 信任）

- 客户端拿到证书后立即调用，验证整链配置
- 服务端：enrollment `issued → active`；换设备场景此刻才吊销旧证书
- 请求体：`{ "slot": "S" }`（身份取自 mTLS 证书 CN/serial）

### POST /revoke  （管理端，管理员鉴权）

```json
{ "serial": "...", "reason": "superseded|key_compromise|decommission" }
```

### GET /crl  （只读，可选）

从 enrollment 表生成签名 CRL，供外部依赖方使用。

## 5. 数据表（PostgreSQL, sqlx）

```sql
CREATE TABLE device_bootstrap_token (
  token_hash   BYTEA PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('auto','approval')),
  device_slot  TEXT NOT NULL,
  created_by   TEXT NOT NULL,
  expires_at   TIMESTAMPTZ NOT NULL,
  used_at      TIMESTAMPTZ
);

CREATE TABLE device_enroll_challenge (
  challenge_id UUID PRIMARY KEY,
  challenge    BYTEA NOT NULL,
  token_hash   BYTEA NOT NULL REFERENCES device_bootstrap_token,
  device_slot  TEXT NOT NULL,
  issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at   TIMESTAMPTZ NOT NULL,
  consumed_at  TIMESTAMPTZ
);

CREATE TABLE device_enrollment (
  serial        TEXT PRIMARY KEY,
  device_slot   TEXT NOT NULL,
  subject_cn    TEXT NOT NULL,
  csr_sha256    BYTEA NOT NULL,
  cert_pem      TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN ('pending','issued','active','rejected','revoked')),
  client_info   JSONB,
  issued_at     TIMESTAMPTZ,
  activated_at  TIMESTAMPTZ,
  revoked_at    TIMESTAMPTZ,
  revoke_reason TEXT
);
-- 同一 slot 只允许一个非终态证书（部分唯一索引）
CREATE UNIQUE INDEX device_slot_one_live ON device_enrollment (device_slot)
  WHERE status IN ('pending','issued','active');
```

## 6. 防抖持久化会话（debounce）

目标：网络抖动/重连时抑制重复状态翻转与重复消息，且重启可恢复。

- **会话标识**：`(serial, device_slot)`，服务端内存 `DashMap` + DB 落盘 `device_session` 表
- **客户端**：指数退避重连（100ms → 30s 上限，jitter），出站消息带单调 `seq`，未 ACK 消息持久化（sled/文件队列），重连后重放
- **服务端**：按 `(serial, seq)` 去重；入站事件经防抖窗口（默认 250ms）合并后再分发；`last_seen_seq` 持久化
- **下线判定**：心跳间隔 5s，3 次未达 → 标记 offline（防抖，不立即广播），恢复后合并为一次状态变更

## 7. 客户端凭证持久化

- `cert.pem` / `key.pem` 落盘，权限 0600
- `server pin`（SPKI sha256）写入本地 trust store（TOFU 首次人工确认后）
- 换设备：管理员吊销旧 serial → 生成新链接 → 新设备走同一流程

## 8. 错误码（注册端点）

| code | 语义 |
|---|---|
| `token_invalid` | token 不存在/过期/已用 |
| `slot_conflict` | slot 已有存活证书（409） |
| `challenge_expired` / `challenge_consumed` / `challenge_mismatch` | 挑战问题 |
| `pop_invalid` | 持有证明验签失败 |
| `fp_mismatch` | TOFU 指纹与服务端实际不符（疑似 MITM） |
| `pending_approval` / `approval_rejected` | approval 型状态 |
