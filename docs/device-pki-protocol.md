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
- 校验：token 有效未用未过期、slot 匹配；approval 型创建 pending 注册单。
  **同 slot 已有 `pending` 单时直接复用**（不返回 409）：否则客户端在 challenge 之后、
  enroll 之前掉线一次就会被自己刚建的 pending 单永久挡死
- slot 已有 `issued`/`active` 证书 → `slot_conflict`（409）
- 挑战绑定三元组 `(token_hash, slot)`

响应：
```json
{ "challenge_id": "uuid", "challenge": "<hex 32B>", "ttl_secs": 60, "kind": "auto", "enroll_id": "uuid" }
```
`enroll_id` 仅 approval 型返回，客户端应**立刻落盘**（配合私钥，见 §7）。

### POST /enroll

请求：
```json
{
  "challenge_id": "uuid",
  "token": "T",
  "slot": "S",
  "csr_der": "<base64>",
  "pop_signature": "<base64 ECDSA/Ed25519>",
  "pop_algorithm": "ecdsa-p256-sha256",
  "tls_cert_seen_fp": "<hex sha256 of server cert seen by client>"
}
```
- `canonical_transcript = challenge || token_hash || slot || sha256(csr_der) || tls_cert_seen_fp`
- 服务端验证：challenge 未过期未消费且三元组匹配、PoP 签名、TOFU 指纹对比
  （**指纹为空或不匹配一律拒绝**，不允许空值绕过）
- 客户端**必须**把 `tls_cert_seen_fp` 钉在真正发送本请求的那条 TLS 连接上：
  若只在另一条探测连接上校验，攻击者可以两条连接出示不同证书，把 token 骗走
- 事务：`consume challenge` + `consume token` + 顶掉同 slot 旧证书 + 插/更新 enrollment
  **同一事务**，任一步失败整体回滚 → token 不会被烧掉，客户端可用同一链接重试

auto 型响应：
```json
{ "status": "issued", "cert_pem": "...", "ca_chain_pem": "...", "serial": "...", "mtls_endpoint": "host:port" }
```

approval 型响应：
```json
{ "status": "pending", "enroll_id": "uuid", "poll_after_secs": 5 }
```

### GET /enroll/{id}  （approval 型轮询）

- `pending` / `rejected` / `approved` / `active`
- 响应含 `mtls_endpoint`（**每个状态都带**），否则 approval 型客户端拿到证书后
  只能拿注册端点去 confirm，必然 404
- 可取性以 `confirm`（激活）为界：
  - `issued` 期间**可重复下载**——服务端在响应体真正送达前就已置位 `downloaded_at`，
    响应在网络上丢失时"一次性"会让设备永久拿不到证书（token 已消费、无路可走）
  - `active` 后返回 410：激活时服务端已把 `cert_pem` 清空，DB 不再长期持有证书

### POST /enroll/confirm  （mTLS listener，业务 CA 信任）

- 客户端拿到证书后立即调用，验证整链配置
- **握手前置条件**：证书必须由本 CA 签发、在有效期内，且在 `device_enrollment`
  里处于 `issued`/`active` 且未被吊销。`pending`（还没批）/`rejected` 阶段就已
  签好的证书**永远连不上**；状态查内存缓存（每 5s 刷新），从未成功刷新过时
  拒绝一切握手（fail-closed）
- **serial 口径**：两侧统一用去掉前导零的规范 hex。DER 编码 INTEGER 会剥掉
  前导零，直接 hex 对比时约 1/256 的证书会永远对不上，吊销就形同虚设
- 服务端：enrollment `issued → active`，同时**清空 `cert_pem`**（证书已交付并被使用）
- 换设备（同 slot 顶掉旧证书）发生在 `POST /enroll` 时刻并立即写入吊销缓存；
  注意 challenge 阶段会把已有 `issued/active` 的 slot 挡成 409，
  所以实际换设备流程是：管理员 `/revoke` 旧 serial → 发新链接
- 请求体：`{ "slot": "S" }`（身份取自 mTLS 证书 CN，与 body 不一致即 401）

### GET /channel  （mTLS listener，WebSocket 隧道）

一条连接同时承担**注册激活**与**双向消息**：升级成功后服务端把 enrollment
推到 `active`（等价于 `POST /enroll/confirm`，幂等），并先发一帧

```json
{ "type": "ready", "status": "active", "serial": "..." }
```

帧协议（JSON 文本帧）：

```text
设备 → 服务端
  {"type":"msg","seq":1,"kind":"heartbeat","payload":"{}"}   // 语义同 POST /messages
  {"type":"ack","up_to":7}                                   // 下行消息处理成功
  {"type":"ping"}

服务端 → 设备
  {"type":"ack_msg","seq":1,"status":"ok","duplicate":false}
  {"type":"down","id":7,"kind":"config","payload":"{}"}
  {"type":"pong"}
  {"type":"err","code":"...","message":"..."}
```

- 上行走同一个 `process_incoming`（收件箱去重 → 业务分发 → 成功才推水位），
  失败回 `err` 帧且**不推水位**，设备应重发同一条
- 下行走同一个队列：服务端在「投递通知 + 2s 兜底轮询」时推 `down` 帧，
  设备处理成功回 `ack` 才标记已投递；没 ack 的在重连后重投
- 心跳：服务端每 20s 发一帧探活，连续 2 个周期无响应即断开；设备侧 15s 主动发 `ping`
- **HTTP 端点保留作为兜底**：设备维持不了长连接时仍可用
  `POST /messages` + `GET /messages/downlink`

### POST /messages  （mTLS listener，设备上行消息）

```json
{ "seq": 1, "kind": "heartbeat", "payload": "{}" }
```

- `seq` 从 1 起单调递增；服务端按 `(serial, seq)` 去重，**重复返回 2xx +
  `{"duplicate": true}`**（幂等，客户端重放不需要担心副作用）
- 水位写入 `device_session.last_seen_seq`，重启后去重仍然有效
- 身份取自 mTLS 证书（CN + serial），不信任请求体；`kind` 1..64 字符、
  `payload` ≤ 64 KiB
- 这是防抖会话层的入口：新消息还会触发"此前被判 offline 的会话恢复 online"

### GET /messages/downlink  （mTLS listener，设备拉取下行消息）

`?after=<上次 ACK 的游标>&limit=<条数，上限 500>`

```json
{ "messages": [ { "id": 7, "kind": "config", "payload": "{}", "created_at": "..." } ],
  "next_after": 7 }
```

### POST /messages/downlink/ack  （mTLS listener）

```json
{ "up_to": 7 }
```

下行语义与上行**对称（至少一次）**：

- 业务方用 `messaging::downlink_publish(...)` / `downlink_publish_to_slot(...)`
  排消息（后者自动解析该 slot 当前存活证书：`issued`/`active`，所以可以在设备
  还没 `confirm` 时先排上）
- **拉取不等于投递**：设备处理成功后才 `ack`；没 ack 的下次还会拿到同一批
- 游标是全局自增 `id`，设备侧落盘（0600），重启后从上次位置继续
- 背压：单台设备未 ACK 超过 1000 条时 `downlink_publish` 报
  `downlink_full`（503），而不是把库写爆；已投递的由 `downlink_cleanup`
  按保留期回收

### POST /admin/tokens  （管理端，Bearer）

```json
{ "kind": "auto|approval", "slot": "S", "ttl_secs": 900, "created_by": "ops@x" }
```
返回一次性明文注册链接（`link`），库中只存 SHA-256。`slot` 只允许 `[A-Za-z0-9._-]`。

> **限速**：所有 `/admin/...` 按对端 IP 计预算（`ADMIN_RATE_LIMIT`，
> 默认 30 次/分钟，`0` = 关闭），超限返回 `429` + `Retry-After`。
> 静态 bearer 没有账号锁定可言，限速是唯一防线。

### GET /admin/enrollments?status=pending  （管理端，Bearer）

注册单列表（含 `client_info`、时间戳、是否已下载）。审批的入口：
没有它，管理员必须先从客户端拿到 `enroll_id` 才能 decide。

### DELETE /admin/enrollments/{id}  （管理端，Bearer）

清理 `pending`（卡死的半截注册）与 `rejected`；`issued/active` 走 `/revoke`，
`revoked` 保留作审计。

### POST /admin/enroll/{id}/decide  （管理端，Bearer）

```json
{ "approve": true, "note": "..." }
```
`approve: false` 时**同时吊销**该注册单在 enroll 阶段就已签好的证书
（`revoked_at = now()`，并立即写入吊销缓存）——否则一张长有效期证书会以
`rejected` 身份留在库里，而 mTLS 握手只查吊销缓存。

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
  enroll_id     UUID,
  downloaded_at TIMESTAMPTZ,
  issued_at     TIMESTAMPTZ,
  activated_at  TIMESTAMPTZ,
  revoked_at    TIMESTAMPTZ,
  revoke_reason TEXT
);
-- 同一 slot 只允许一个非终态证书（部分唯一索引）
CREATE UNIQUE INDEX device_slot_one_live ON device_enrollment (device_slot)
  WHERE status IN ('pending','issued','active');

-- 设备上行消息收件箱：跨重启去重 + 至少一次处理 + 审计
CREATE TABLE device_message_inbox (
  serial       TEXT NOT NULL REFERENCES device_enrollment(serial),
  seq          BIGINT NOT NULL CHECK (seq > 0),
  device_slot  TEXT NOT NULL,
  kind         TEXT NOT NULL,
  payload      TEXT NOT NULL,
  attempts     INT NOT NULL DEFAULT 0,
  received_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  processed_at TIMESTAMPTZ,
  last_error   TEXT,
  PRIMARY KEY (serial, seq)
);
CREATE INDEX device_message_inbox_unprocessed
  ON device_message_inbox (received_at) WHERE processed_at IS NULL;
```

> 迁移文件在 workspace 根 `migrations/0006_device_pki.sql`（消息收件箱在
> `0007_device_messaging.sql`），与 api/db 共用同一份。
> 曾经 device-pki 自带一套以 `0001_` 开头的迁移，与根目录 `0001_init.sql` 版本号冲突，
> 两个二进制共用一个库时 sqlx 会直接报 `VersionMismatch`/`VersionMissing` 让进程起不来。

## 6. 防抖持久化会话（debounce）

目标：网络抖动/重连时抑制重复状态翻转与重复消息，且重启可恢复。

- **会话标识**：`(serial, device_slot)`，服务端内存 + DB 落盘 `device_session` 表；
  **启动时把水位读回内存**，重启后去重立刻生效
- **客户端**：指数退避重连（100ms → 30s 上限，jitter），出站消息带单调 `seq`，
  先落 sled 再发送，**收到服务端 ACK 才删除**；重连后按 seq 逐条真正重发
- **服务端**：`POST /messages` 的处理顺序是 **收件箱去重 → 业务分发 → 成功才推水位**：

  | 情况 | HTTP | `status` | 收件箱 | 水位 |
  |---|---|---|---|---|
  | 首次到达、handler 成功 | 2xx | `ok` | `processed_at = now()` | 推进 |
  | handler 失败 / 超时 | 5xx | — | `attempts+1`、`last_error` | **不推**（客户端重发自愈） |
  | 重复 seq 且已处理 | 2xx | `duplicate` | 不变 | 不变 |
  | 同条正在处理 | 2xx | `in_flight` | 不变 | 不变 |
  | 没有订阅方的 kind | 2xx | `no_handler` | 记为终态 + 原因 | 推进 |

  收件箱是 `device_message_inbox`，主键 `(serial, seq)`，因此去重**跨进程重启有效**，
  并留下审计轨迹（谁、何时、什么载荷、第几次尝试、失败原因）
- **业务分发**：`qe-device_pki::messaging::Dispatcher` 按 `kind` 路由到 handler，
  同 kind 可挂多个（全部成功才算成功），`"*"` 是通配订阅；handler 有超时保护。
  `qe-device-pki` 是 lib crate，别的服务（如 `crates/api`）依赖它后
  直接 `dispatcher.subscribe("telemetry", ...)` 即可接入
- **下线判定**：后台每 5s 跑一轮 `offline_tick`，把「连续 3 次未达心跳」的会话
  推进防抖窗口标记 offline（不立即广播），恢复后合并为一次状态变更
- **下行（服务端 → 设备）**：`device_downlink` 队列 + `GET .../downlink` 拉取 +
  `POST .../ack`。同样是至少一次：**处理成功才 ack**，否则下次重投；
  设备侧游标落盘，重启后继续。与上行的共同原则是
  **"确认成功才推进水位/游标"**，任何一边都不允许"收到即忘"
- **节奏**：客户端成功一轮后按心跳间隔（5s）再探，而不是退避下限 —— 否则
  confirm 会被打成 ~10 次/秒，违反心跳约定并冲击服务端防抖

## 7. 客户端凭证与信任持久化

- `cert.pem` / `key.pem` 落盘，权限 0600（每次写都重新 `chmod`，覆盖已有文件也生效）
- **进行中的 approval 注册**落 `pending/<slot>.json`：设备私钥 + `enroll_id`。
  CSR 一旦提交私钥就换不回来，若等到拿到证书才存盘，等待审批期间进程一退出
  这台设备就永久注册不上（token 已消费）。中断后用 `poll --slot <slot>` 续跑，
  不需要原注册链接
- TOFU 首次连接必须**展示指纹并人工核对**（交互确认或显式 `--trust`），
  不能是"看都不看就信"
- 注册链接里含一次性 token，客户端支持从 **stdin** 读链接（`enroll -`），
  避免它留在 shell history / 进程参数里
- 注册通道：客户端把已校验的证书指纹钉进真正传 token 的 TLS 连接
- mTLS 通道：用 `ca_chain_pem` 作为信任根做链验证 + 主机名校验，
  不允许 `danger_accept_invalid_certs`
- 换设备：管理员吊销旧 serial → 生成新链接 → 新设备走同一流程

## 8. 错误码（注册端点）

| code | 语义 |
|---|---|
| `token_invalid` | token 不存在/过期/已用 |
| `slot_conflict` | slot 已有存活证书（409） |
| `challenge_expired` / `challenge_consumed` / `challenge_mismatch` | 挑战问题 |
| `pop_invalid` | 持有证明验签失败 |
| `fp_mismatch` | TOFU 指纹与服务端实际不符（疑似 MITM） |
| `pending_approval` / `approval_rejected` | approval 型状态（`rejected`/`revoked` 后轮询） |
| `already_downloaded` | 证书已激活交付，不可再取（410） |
| `enrollment_not_found` | 注册单不存在或已被管理端清理（404） |
| `unauthorized` | 管理端 Bearer 缺失或错误（401） |
| `rate_limited` | 管理端超出每 IP 每分钟预算（429 + `Retry-After`） |
| `bad_request` | 请求体不合法，例如 `slot` 不在白名单内（400） |
| `internal` | 服务端内部错误（500） |

> `pending` **不是错误码**：challenge/enroll 轮询阶段它以 `{"status":"pending"}` 正常返回 200。
