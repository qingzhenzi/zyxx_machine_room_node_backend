# 浏览器查看虚拟机画面（noVNC）设计

> 状态：阶段 1 已随「VM 远程控制」落地（截图轮询）；阶段 2（实时 noVNC）为设计稿，未实现。
> 关联：[remote-vm-control.md](remote-vm-control.md)、[../migrations/0031_machine_command_results.sql](../migrations/0031_machine_command_results.sql)

## 1. 问题

虚拟机跑在房间机器（qe-agent 所在宿主机）的 libvirt 上，VNC 端口默认只监听
`127.0.0.1`（见 `virt-core` 的 `display` 渲染：`listen=127.0.0.1 autoport=yes`）。
控制节点的浏览器与房间机器之间**没有直连通路**：
- 反向代理不覆盖房间机器；
- 把 VNC 端口暴露到局域网等于绕过认证，不可接受。

因此「浏览器看 VM 画面」需要一个**受控的隧道**或**帧代理**。

## 2. 阶段 1（已实现）：截图轮询

`vm.command` 的 `screenshot` op 让 agent 用 libvirt Stream 截一张 PNG，
以 base64 放进 `machine_command_results.result`：

```json
{"format":"png","image_base64":"iVBOR...","size_bytes":12345}
```

前端在机器详情的「虚拟机」面板里用 `<img src="data:image/png;base64,...">` 显示，
并提供「刷新」按钮。优点：零新协议、复用 mTLS 命令通道、无需 TCP 隧道；
缺点：单帧、不适配交互（键鼠注入）。

适用：演示「看画面」、排障截图、低帧率监看。

## 3. 阶段 2（设计稿）：实时 noVNC

### 3.1 目标链路

```
浏览器 noVNC (RFB over WebSocket)
  └─ wss://control-node/api/v1/admin/machines/{serial}/vnc/ws   (JWT + vm:control)
       └─ qe-api：WS ↔ 内部隧道多路复用
            └─ device-pki：已建立的 agent mTLS WebSocket `/channel`
                 └─ agent：本地 TCP 127.0.0.1:<vnc-port>（libvirt 分配）
```

### 3.2 协议扩展（device-pki `/channel`）

现有 `/channel` 是 agent 主动连到 device-pki 的 mTLS WebSocket，用于消息推送。
阶段 2 在其上增加**双向二进制隧道帧**：

- JSON 文本帧：控制消息（保持现有格式，新增 `type:"vnc.open"/"vnc.close"/"vnc.error"`，
  带 `session_id`、`serial`、`vm`）；
- 二进制帧：`[u8 kind][u32 session_id][payload]`，`kind=1` 为 VNC 数据（双向）；
- 背压：队列高水位时丢弃输入事件（键鼠），画面帧不丢；单 session 上限
  （如 8 个并发、每 session 2MiB 缓冲）。

### 3.3 控制节点内部中转

qe-api 与 device-pki 是同一网络内的两个进程（compose 同网段）。两种实现：
1. **Redis pubsub 中转**（推荐先做）：qe-api 订阅 `vnc:{serial}` 频道，device-pki
   把 agent 隧道帧 publish 到该频道；qe-api 把浏览器 WS 帧 publish 回
   `vnc:{serial}:to-agent`。现有 compose 已有 Redis，延迟可接受（同机 <1ms）。
2. **device-pki 暴露内部 WS**：qe-api 以内部 token 连接 device-pki 的
   `/internal/vnc/{serial}`，双向转发。实现更干净，但要新增内部认证。

### 3.4 前端

- `@novnc/novnc`（npm）的 `RFB` 组件：`new RFB(canvas, url, {credentials:{password}})`；
- `noVNC` 需要 RFB 密码：libvirt `display.password` 已在 `virt-core::GraphicsInfo`
  里有字段，agent 在 `vnc.open` 回应里带一次性密码（不要持久化到 DB）；
- 权限：`vm:control`；审计：打开/关闭各一条 `vm.vnc.open/close`；
- 浏览器端不接触 VNC 端口与证书，只连控制节点 WSS。

### 3.5 安全约束

- 隧道只允许 `vm:control` 用户打开，且 session 绑定 serial+vm，禁止转发任意 host/port；
- agent 侧用白名单校验（只允许本机 127.0.0.1 且端口来自当前 VM 的 `graphics()`）；
- 一次性 VNC 密码 + 会话超时（如 30 分钟无输入自动关闭）；
- 审计记录用户、serial、vm、起止时间、字节数。

### 3.6 工作量与验收

| 阶段 | 内容 | 估时 | 验收 |
|------|------|------|------|
| 1 | 截图轮询（已做） | 0.5d | 机器详情能看到 VM 截图并刷新 |
| 2a | `/channel` 二进制隧道 + agent 本地 TCP 转发 | 2–3d | 单机 libvirt VM 用 wscat 打通 RFB 握手 |
| 2b | qe-api WSS 端点 + Redis 中转 | 2–3d | 浏览器 noVNC 连上并看到画面 |
| 2c | 权限/审计/背压/多会话 | 1–2d | 权限负测、断线重连、双会话 |

## 4. 明确不做的部分

- 不做浏览器直连 agent 的 VNC 端口（安全）；
- 不在本阶段做音频/剪贴板/USB 重定向（noVNC 支持但非教学必需）；
- 不替代 `lan_video` 的低延迟视频采集链路（那是教学广播场景，另一条链路）。

## 5. 实施定稿（2026-11-08，阶段 2 落地版本）

为避开浏览器直连设备自签证书/房间网络不可达的问题，采用 **qe-api 代理 + device-pki 内部中转**：

```
浏览器 noVNC (RFB over WebSocket)
  └─ ws://control/api/v1/admin/machines/{serial}/vnc/ws?session&ticket   (qe-api，JWT 会话)
       └─ qe-api 内存 session 表（ticket/过期/一次性），升级后作为 WS 中继
            └─ device-pki 内部明文监听 127.0.0.1:4442 /internal/vnc/{session}
                 └─ device-pki VNC 中继表（session 配对）
                      └─ agent mTLS WS /vnc/{serial}?session   （agent 主动连出）
                           └─ agent → 127.0.0.1:<libvirt VNC 端口> TCP 桥
```

- **agent**：新增 `vm.command vnc.info {name}`（返回 vnc host/port/password）；
  新增 downlink kind `vnc.open {session_id, vm}`：解析 libvirt graphics → 用设备证书连
  `wss://<DEVICE_PKI_MTLS>/vnc/<serial>?session=<id>` → 本地 TCP 127.0.0.1:port →
  WS 二进制帧双向泵；空闲 30min/错误即关闭。
- **device-pki**：`mtls_router` 加 `GET /vnc/{serial}`（证书 serial 必须等于路径 serial）；
  新增内部明文监听 `INTERNAL_ADDR`（默认 127.0.0.1:4442）提供 `GET /internal/vnc/{session}`
  （header/query `x-internal-token` 与 env `INTERNAL_TOKEN` 比对）；按 session 配对
  agent/internal 两端，30s 配对超时、30min 空闲超时，二进制帧透传。
- **qe-api**：env `DEVICE_PKI_INTERNAL_URL`、`DEVICE_PKI_INTERNAL_TOKEN`；
  `POST /admin/machines/{serial}/vnc/session`（vm:control）→ 下发 vnc.info 命令拿密码/端口
  → 生成 session_id+ticket（内存 TTL 60s）→ 返回 `{session_id,ticket,ws_url,password,vm}`；
  `GET /admin/machines/{serial}/vnc/ws?session&ticket`：校验 ticket（一次性）→ 升级 WS →
  连 device-pki internal WS → 双向中继（WS 会话与中继任务同生命周期）。
- **前端**：`@novnc/novnc@1.7`；VM 行「控制台」→ session API → 新建 RFB(canvas, ws_url,
  {credentials:{password}})；弹窗内 canvas 自适应；关闭/断线/权限错误 toast。
- **验收**：本机 session VM（qemu:///session，VNC 随机端口）全链路：API 建 session →
  浏览器/脚本完成 RFB 握手并收到 ServerInit/MsgServerCutText 帧；输入帧能回传（键鼠）；
  跨用户/跨机器仍走同一链路（真机验收待环境）。

## 6. 实现状态（2026-11-08）

阶段 2 控制节点侧已落地（agent 侧由 teammate 并行实现）：

| 组件 | 实现 | 测试 |
| --- | --- | --- |
| device-pki agent 隧道 | `mtls_router` `GET /vnc/{serial}?session=`（证书 serial 必须等于路径 serial；session `[A-Za-z0-9_-]{16,64}`） | 配对状态机单测（先到等待/后到交接/同端重复/过期清理/session 校验/token 常量时间比较） |
| device-pki 内部中继 | 内部明文 listener `INTERNAL_ADDR`（默认 127.0.0.1:4442，未配置 `INTERNAL_TOKEN` 则不启动并 warn）`GET /internal/vnc/{session}?token=` | 同上 |
| VNC 配对/透传 | `AppState.vnc` 配对表；30s 配对超时、30min 空闲超时；二进制帧双向透传（≤2MiB/帧，文本帧丢弃）；同一 session 仅一对 | 状态机单测 |
| qe-api 会话 | `POST /api/v1/admin/machines/{serial}/vnc/session`（`vm:control`）→ 下发 `vm.command vnc.info`（≤5s）→ 一次性 session/ticket（TTL 60s）→ `{session_id,ticket,vm,password,expires_at}` | ticket 一次性/过期/serial 绑定、URL 构造单测 |
| qe-api WS 代理 | `GET /api/v1/admin/machines/{serial}/vnc/ws?session=&ticket=`（ticket 鉴权）→ downlink `vnc.open` → 连 `ws://DEVICE_PKI_INTERNAL_URL/internal/vnc/{session}` → 双向中继；连接失败 502 | 同上 |
| 环境变量 | device-pki：`INTERNAL_ADDR`、`INTERNAL_TOKEN`；qe-api：`DEVICE_PKI_INTERNAL_URL`、`DEVICE_PKI_INTERNAL_TOKEN`（compose/.env.example 已补） | — |

**本机可在无真实 agent 的机器上验证**：启动 device-pki 后 `GET http://127.0.0.1:4442/internal/vnc/{session}?token=wrong` → 403；
token 正确但未升级 WS → 400/426（listener 正常）。真实 RFB 全链路需 agent + libvirt VNC 环境（见 §5 验收）。
