# qe-device-pki

设备身份注册与 mTLS 通讯的服务端。协议契约见 [`docs/device-pki-protocol.md`](../../docs/device-pki-protocol.md)。

## 特性

- **一次性 token → 持久证书**：`auto` 型即时签发；`approval` 型由管理员审批后下载
  （激活前可重取，`confirm` 激活后服务端清空 `cert_pem`）
- **持有证明（PoP）**：客户端用 CSR 私钥签 `canonical_transcript`，防止 token 被截获后换别人的 CSR
- **挑战值**：60s TTL、绑定 `(token_hash, slot)`、一次性消费，抵抗重放
- **注册全流程一个事务**：消费 challenge/token + 顶掉旧证书 + 落库同生共死，
  任何一步失败 token 都不会被烧掉，客户端可用同一链接重试
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
| `MTLS_TLS_CERT` / `MTLS_TLS_KEY` | 业务端点证书/私钥（**必须能被 `CA_CERT` 链验证**，客户端拿 `ca_chain_pem` 当信任根） |
| `CA_KEY` / `CA_CERT` | 内部 CA 签名密钥/证书，默认 `certs/ca-key.pem`、`certs/ca.pem` |
| `ADMIN_TOKEN` | 调用 `/admin/...`、`/revoke` 的 bearer token |
| `ADMIN_RATE_LIMIT` | 管理端每 IP 每分钟预算，默认 `30`；`0` = 关闭（前面有反代限速时才考虑） |
| `PUBLIC_ENROLL_ENDPOINT` | 对外公布的注册地址，用于拼注册链接；默认把监听地址里的 `0.0.0.0` 换成 `localhost` |
| `PUBLIC_MTLS_ENDPOINT` | 下发给客户端的 mTLS 地址，同上 |
| `NOTIFY_REDIS_URL` | **多实例部署**：设置后启用跨进程下行通知（需 `--features redis-notify`）；不设置则完全不连 Redis |
| `NOTIFY_REDIS_CHANNEL` | 跨进程通知频道，默认 `qe-device-pki:downlink` |

### 多实例部署

单实例时下行通知是进程内的 `Notify`（零成本）。多实例时如果不做跨进程通知，
业务方实例 A 投递的消息要等设备连着的实例 B 轮询到（最多 2s 延迟）。
启用跨进程通知后延迟与单实例一致：

```bash
cargo build --release -p qe-device-pki --features redis-notify
NOTIFY_REDIS_URL=redis://redis:6379 ./qe-device-pki
```

实现上只用**一条订阅连接**：所有设备的通知都发到同一个频道，载荷是 `serial`，
进程收到后用进程内的 `Notifier` 唤醒对应连接 —— 连接数 O(1) 而不是 O(设备数)。
通知发送失败/订阅断线都不会丢消息（2s 兜底轮询会捞回来），只是延迟退化。

## 端点

注册 listener（单向 TLS）：

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/enroll/challenge` | 下发挑战值；approval 型创建（或**复用**）pending 单 |
| POST | `/enroll` | 提交 CSR + PoP；auto 直发证书，approval 返回 pending |
| GET | `/enroll/{id}` | approval 轮询；approved 后下载（激活前可重取，激活后 410） |
| POST | `/admin/tokens` | 生成一次性注册链接 `link`（Bearer） |
| GET | `/admin/enrollments` | 注册单列表，`?status=pending` 过滤（Bearer） |
| DELETE | `/admin/enrollments/{id}` | 清理 `pending`/`rejected` 注册单（Bearer） |
| POST | `/admin/enroll/{id}/decide` | 批准/拒绝（Bearer）；拒绝会**同时吊销**已签证书 |
| GET | `/crl` | 签名 CRL |

业务 listener（mTLS，客户端证书必需）：

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/enroll/confirm` | 回连验证，`issued → active` 并清空 `cert_pem`，通讯打通 |
| GET | `/channel` | **WebSocket 隧道**：建连即激活注册，之后双向消息走这一条连接（帧协议见契约 §4） |
| POST | `/messages` | 设备上行消息；收件箱去重 + 业务分发 + 成功才推水位 |
| GET | `/messages/downlink` | 设备拉取下行消息（`?after=&limit=`） |
| POST | `/messages/downlink/ack` | 设备确认已处理（`{"up_to": id}`） |
| POST | `/revoke` | 吊销（Bearer） |

所有 `/admin/...` 都按对端 IP 限速（429 + `Retry-After`）：静态 bearer 没有
账号锁定，限速是唯一防线。

## 上行消息：接入业务处理

`qe-device-pki` 是 **lib crate**，消息处理这层（`messaging`）不依赖任何 HTTP
细节，别的服务直接依赖本 crate 就能订阅自己关心的类型：

```rust
use qe_device_pki::messaging::{Dispatcher, Message};
use std::sync::Arc;
use std::time::Duration;

let dispatcher = Arc::new(Dispatcher::new(Duration::from_secs(10)));
dispatcher.subscribe("telemetry", |m: Message| async move {
    // m.serial / m.slot / m.seq / m.kind / m.payload
    // 返回 Err → 客户端会重发同一条，下次自愈
    Ok(())
});
dispatcher.subscribe_all(|m: Message| async move { /* 通配：审计/指标 */ Ok(()) });
```

语义（契约 §6 有完整表格）：

- **收件箱去重**：`device_message_inbox` 主键 `(serial, seq)`，跨重启有效；
  记 `attempts`/`last_error`/`processed_at`，失败的重发会自愈
- **成功才推水位**：handler 失败或超时 → 5xx 且不动 `last_seen_seq` → 客户端重发
- **没有订阅方的 kind**：回 `no_handler` 并记为终态（重试也不会变，别让客户端死循环）
- **handler 要快速返回**（默认 10s 超时）；慢活请自己 spawn

离线判定由 `messaging::spawn_offline_detector` 驱动（每 5s 一轮）——
没有它，`SessionManager::heartbeat_miss` 永远没人调用，设备只会"回到在线"
而永远不会被判 offline。

### 下行（服务端 → 设备）

```rust
use qe_device_pki::messaging;

// 按 slot 投递给当前存活证书（issued/active），设备没 confirm 时也能先排上
let id = messaging::downlink_publish_to_slot(&pool, "slot-1", "config", r#"{"mode":"a"}"#).await?;
// 或者直接指定 serial
let id = messaging::downlink_publish(&pool, &serial, "slot-1", "config", "{}").await?;
```

设备侧拉取 `/messages/downlink?after=<游标>` → 处理 → 成功后 `/ack`。
**拉取不等于投递**：没 ack 的下次还会拿到（至少一次）。

背压：单台设备未 ACK 超过 1000 条时报 `downlink_full`（503）而不是无限堆积；
已投递的由 `messaging::downlink_cleanup(pool, keep)` 按保留期回收。

## 关键实现说明

**握手可用性检查**：`tls::DeviceCertVerifier` 在每次 mTLS 握手时查内存里的
`CertStatusCache`（后台每 5s 全量刷新并**整体原子替换**，`/revoke`、审批拒绝与
「顶掉旧证书」即时写入）。除了吊销，还要求证书在 `device_enrollment` 里处于
`issued`/`active` —— 否则 `pending`/`rejected` 阶段就已签好的证书只要 PEM 泄露
就永远可用。**fail-closed**：一次都没成功刷新过时拒绝所有握手。
同步回调里不碰 DB —— 早期版本用 `block_in_place` 查库，单线程 runtime 下
`block_in_place` 会 panic 并**降级放行**，等于单线程部署时吊销根本不生效。

**serial 口径**：DER 编码 INTEGER 会剥掉前导零，所以库里记的 20 字节与证书里
读出来的可能不等长（约 1/256）。两侧统一走 `ca::canonical_serial_hex`，
否则那些证书的吊销状态永远对不上。

**为什么两个 listener**：rustls 在握手阶段就要确定是否要求客户端证书，运行时切换会让状态机很难维护。

**CA 持久化**：`Ca::load(key_pem, cert_pem)` 从磁盘加载。`Ca::generate_self_signed` 仅供开发测试——生产环境重启后必须加载同一份 CA，否则已签发证书无法验证。

**迁移**：`sqlx::migrate!("../../migrations")`，与 api/db crate 共用 workspace 根
那份迁移（device-pki 的表在 `0006_device_pki.sql`）。两套各自以 `0001_` 开头的
迁移目录指向同一个库时，sqlx 会因 version/checksum 冲突让后启动的进程起不来。

## 测试

```bash
cargo test -p qe-device-pki                    # 56 个单元测试
./scripts/test-integration.sh                  # 另加 9 个端到端用例（真实 PostgreSQL，含 WS 隧道）
```
