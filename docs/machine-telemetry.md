# 机器遥测契约（room-agent → device-pki / qe-api）

> 目标：房间内的「子机器端」周期性采集主机清单 + 健康结论 + 诊断结果，上报到服务端，
> 管理控制台在「机器总览」里看到每台机器的实时状态与历史快照。
>
> 本文是 **v1.3 契约**；`room-agent` 实现、`qe-device-pki` 的 mTLS handler 与
> `qe-api` 的管理端接口都以本文为准。相对 v1/v1.1/v1.2 的变化见 §0（v1.2 为 device_id 身份勘误，
> 身份模型以 `docs/device-pki-protocol.md` v0.2 为准；v1.3 增加 WOL 的 MAC 字段）。

## 0. v1.1 相对 v1 的变化

| 变化 | v1 | v1.1 |
|---|---|---|
| 默认认证 | fleet Bearer token（`MACHINE_AGENT_TOKEN`） | **设备证书 mTLS**（`POST {DEVICE_PKI_MTLS_URL}/machine/report`）；`REPORT_MODE=bearer` 保留旧路径 |
| 采集频率 | 固定一档（默认 15s，10..=3600） | **两档**：fast（内置指标，默认 10s，5..=60）与 full（含外部工具，默认 300s，60..=3600） |
| payload `tier` | 无 | `"fast"` / `"full"`（`machine_snapshots.tier`，0016） |
| 频率配置 | 只能改本地 env | 服务端 `agent_configs` 下发：**global → area → room → device** 合并（0017） |
| 契约版本 | `schema: 1` | 仍为 `schema: 1`（新增字段向后兼容；旧 agent 不带 `tier` 按 full 处理） |

### 0.1 v1.2（2026-11-07）：device_id 身份勘误（不破坏 v1.1）

v1.1 文中"证书 CN 就是 device_slot"的表述已过时；HEAD 起设备身份改为 **`device_id`（UUID）**，
详细协议见 `docs/device-pki-protocol.md` v0.2。本节为对 v1.1 的勘误，`schema` 版本号仍为 `1`：

- **证书身份**：新签发证书的 Subject CN = `device_id`（UUID）；`device_slot` 降级为房间内
  绑定名/展示名，由服务端按证书 serial 反查 `device_enrollment.device_slot` 得到。
  历史证书（CN=slot）仍走 serial/slot 兼容回退，并回填 `device_id`（迁移 0027/0028）。
- **请求体 `device_slot`**：mTLS 入口可省略；若携带，必须等于服务端解析出的**展示 slot**
  （不是证书 CN），否则 400。只有 Bearer 兼容入口仍强制要求 `device_slot`。
- **服务端注入/返回 `device_id`**：mTLS 入口落库前向快照 payload 注入 `device_id`，
  202 响应在既有字段外增加 `device_id`：
  `{snapshot_id, serial, device_id, device_slot, received_at}`。客户端忽略未知字段即可，向后兼容。
- **`/agent/config` 的 device 层匹配**：用解析出的 `device_slot` 匹配 `agent_configs.scope_key`，
  不是证书 CN。
- v1.1 的 `tier`（fast/full）、两档间隔与服务端下发语义完全不变，无需迁移动作。

### 0.2 v1.3（2026-11-07）：`sections.agent.host.macs`（Wake-on-LAN，新增字段）

- 内置 host 采集新增 **`sections.agent.host.macs`**：网卡 MAC 字符串数组，供控制台
  「机器详情 → 网络唤醒」寻找同二层网络内的目标机。
  - 来源：Linux `/sys/class/net/<iface>/address`；
  - **只采集物理网卡**：`/sys/class/net/<iface>/device`（PCI/USB/virtio 设备）或
    `phy80211`（无线）存在才算；bridge/veth/docker0/tun/tailscale 等虚拟接口不会
    成为 WoL 目标（实测宿主机曾上报 16 个虚拟网卡，全是无用 MAC）；
  - 规则：跳过回环接口（`lo`/`lo0`）、跳过全零地址、统一转**小写冒号格式**
    （`aa:bb:cc:dd:ee:ff`）、**排序 + 去重**；读不到的接口直接忽略（数组可为空，
    例如容器内只有 veth 时返回空数组——这是有意的保守行为）；
  - 示例：`["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:02"]`；
  - fast/full 两档**都会**上报（host 是内置采集，不依赖外部工具）；
  - 该 crate 没有「隐私模式/脱敏」开关：MAC 是局域网唤醒的必需字段，只在管理端可见；
    如果未来引入脱敏模式，应把 `host.macs` 一并脱敏或省略，并在此处补充说明。
- 消费方：`qe-api` 的 `POST /api/v1/admin/machines/{serial}/wake`（权限 `device:wake`，
  迁移 `0030_wol_permission.sql`）读取**最新一条**快照的这个字段并发 magic packet；
  链路与限制见 `docs/wol.md`。
- `schema` 版本号仍为 `1`（纯新增字段，旧 agent 不上报时控制台按「无 MAC」处理）。

### 0.4 v1.5（2026-11-14）：机器实时 WS 通道 + 长操作进度（不破坏 v1.x）

- **mTLS WS `/machine/realtime`**：agent 维持一条长连接承载实时样本与长操作进度，
  替代「每样本一次 HTTP POST」；服务端在有人订阅时下发 `want {interval_ms}`，
  最后一个观察者离开即发 `idle`。连不上/断开时 agent 自动回退 v1.4 的 HTTP
  `/metrics` + `telemetry.live` 路径（开关 `ROOM_AGENT_REALTIME_WS=0` 可强制回退）。
  帧协议见 room_node `crates/device-pki/src/realtime.rs`。
  注意：旧版 `ROOM_AGENT_LIVE_PUSH=1`（无条件常推）与 WS 按需模型冲突，设置后 agent
  会禁用 WS 通道、继续用 HTTP 常推。
- **长操作进度**：agent 在 `op_progress` 帧里上报阶段（`accepted` / `cleanup_done` /
  `scheduled` / `rejected`，可带 percent/message）；device-pki 与实时样本共用广播，
  qe-api 以 SSE `op` 事件（`LiveOpProgressEvent`）转发，控制台据此显示“执行中”，
  不必只靠 1s 轮询命令结果。
- **在线心跳顺带增强**：WS 建连与客户端 ping 都会刷新 `device_session` 活动，
  只跑 machine 角色的设备也有一条真实长连接作为在线依据。

### 0.3 v1.4（2026-11-14）：实时指标按需推流 + 在线判定自适应窗口（不破坏 v1.x）

- **在线/新鲜窗口**：`MachineSummary` 的 `online` / 新增 `agent_online` 不再用固定 5 分钟，
  而是 `max(3 × sections.agent.fast_interval_secs, 30s)`；同时返回 `report_age_secs`、
  `stale_after_secs`，`report_stale` 沿用旧字段名但语义变成「超过该窗口」。
  读不到 fast 间隔（旧 agent / 无该字段）按 10s 兜底。详见
  `docs/machine-realtime-observability.md` §2。
- **实时指标按需推流**：agent 默认不再常驻 POST `/metrics`。浏览器打开
  `GET /api/v1/admin/machines/{serial}/metrics/stream?interval_ms=…` 时，qe-api 经
  `device_downlink` 下发 `telemetry.live {enabled, interval_ms, expires_at_ms}`；
  `qe-agent-node` 写入 `<data_dir>/telemetry.live.json`，遥测线程在 TTL 内按间隔采样并
  推送，过期自动停止（浏览器断开后最多 30s）。旧行为保留开关：
  `ROOM_AGENT_LIVE_PUSH=1` 表示无条件持续推送（排障/兼容）；`ROOM_AGENT_LIVE_INTERVAL_MILLIS`
  仍是本地采样基准间隔（默认 500，范围 100–5000）。
- **会话落库**：`POST /machine/report` 同时节流刷新 `device_session`（15s），
  只跑 machine 角色、不拉下行的设备也有会话行与在线事件。
- **事件默认值**：`DEVICE_PRESENCE_NOTIFY_ONLINE` 默认改为 `true`（恢复在线也产生
  `device.online` 事件）；仍受唯一键去重与离线判定的 250ms 去抖约束。

## 1. 角色与链路

```text
room-agent（每个房间机器）
   ├─ fast 档（默认 10s）：内置 /proc、/sys、df、hwmon —— 不做能力采集
   ├─ full 档（默认 300s）：内置采集 + 安全 / 容器 / 虚机（进程内库直调）+ platform-health / platform-diagnose
   ├─ 安全扫描独立规则（默认 300s，0 = 跟随 full 档）：刷新即经 mTLS 实时回传 full 快照
   ├─ 启动/运行期：mTLS GET {DEVICE_PKI_MTLS_URL}/agent/config 拉两档间隔
   └─ mTLS POST {DEVICE_PKI_MTLS_URL}/machine/report        ──► qe-device-pki ──► PostgreSQL
                                                                            │
                        （兼容路径）Bearer POST /api/v1/machine/report ──► qe-api ──┘
                                                                            │
                       管理控制台 /machines ◄── /api/v1/admin/machines ─────┘
```

- **默认（mTLS）**：身份来自设备证书（证书 CN = `device_id` UUID，`device_slot` 由服务端按
  serial 反查 `device_enrollment` 得到；旧证书 CN=slot 走兼容回退），请求体不需要 `Authorization`；
  `device_slot` 可省略，若带上则必须与服务端解析出的展示 `device_slot` 一致（见 §0.1）。
- **兼容（Bearer）**：`REPORT_MODE=bearer` 时继续 `POST /api/v1/machine/report` +
  `Authorization: Bearer <MACHINE_AGENT_TOKEN>`；服务端按 `device_slot` 找
  `device_enrollment`（`status` 为 `issued`/`active`）。
- 上报同时刷新设备在线状态（device-pki 侧写 `device_session`），离线判定沿用 P3 的窗口。

## 2. 上报端点

### 2.1 默认：device-pki mTLS `POST /machine/report`

- 监听器：device-pki 的 mTLS listener（`DEVICE_PKI_MTLS_URL`，如 `https://node:4441`）。
- 请求体：`application/json`，≤ 1 MiB（§3）。
- 成功：`202 Accepted`：

```json
{
  "snapshot_id": 123,
  "serial": "4f40ccef...",
  "device_id": "0f0f7f0e-...-uuid",
  "device_slot": "room-101",
  "received_at": "2026-10-31T14:53:02Z"
}
```

- 幂等：同一 `(serial, collected_at)` 重复上报只保留一条。
- 错误：`400` 校验失败 / `device_slot` 与证书解析出的展示槽位不一致；`401` 证书无存活注册记录；`500` 内部错误。
- 每台机器保留最近 `MACHINE_SNAPSHOT_KEEP` 条（默认 100，10..=1000）。

### 2.2 兼容：qe-api Bearer `POST /api/v1/machine/report`

- 认证：`Authorization: Bearer <MACHINE_AGENT_TOKEN>`（服务端 `MACHINE_AGENT_TOKEN`）。
  未配置 token 时返回 `503 machine_report_disabled`。
- 请求体与 mTLS 入口完全一致，但 `device_slot` **必填**（没有证书可推断身份）。
- 响应/幂等/保留策略同 §2.1；错误：`400` 校验失败、`401` token 错误、
  `404` 槽位不存在、`503` 未启用。

## 3. 上报 payload

```json
{
  "schema": 1,
  "device_slot": "room-101",
  "agent_version": "0.1.0",
  "collected_at": "2026-10-31T14:53:01Z",
  "hostname": "cachyos-qingzhenzi",
  "os_name": "CachyOS Linux rolling",
  "kernel_version": "6.18.42-1-cachyos-lts",
  "tier": "full",
  "health": {
    "status": "ok",
    "score": 96,
    "issues": [
      { "severity": "warning", "code": "disk.usage", "message": "/ usage at 88%", "detail": "..." }
    ]
  },
  "metrics": {
    "cpu_usage_percent": 14.7,
    "memory_usage_percent": 42.1,
    "disk_usage_percent": 61.2,
    "uptime_secs": 3280
  },
  "sections": {
    "platform_health": { "schema": "platform-health/v2", "...": "原样 JSON（仅 full）" },
    "diagnose": { "verdict": "...", "...": "platform-diagnose --json 原样（仅 full）" },
    "security": { "summary": { "pass": 39, "warn": 4, "fail": 1 }, "raw": { } },
    "containers": [ { "name": "py-api", "image": "...", "state": "running", "status": "Up 2 hours" } ],
    "vms": [ { "name": "debian12", "state": "running", "vcpus": 2, "memory_bytes": 2147483648 } ],
    "agent": {
      "agent_version": "0.1.0",
      "tier": "full",
      "report_mode": "mtls",
      "interval_secs": 300,
      "fast_interval_secs": 10,
      "full_interval_secs": 300,
      "tools": { "platform_health": "0.1.0", "platform_diagnose": "0.1.0" },
      "host": { "machine_id": "...", "macs": ["aa:bb:cc:dd:ee:ff"] }
    }
  }
}
```

字段约束：

| 字段 | 必填 | 说明 |
|---|---|---|
| `schema` | 是 | 固定 `1`（未知版本 400） |
| `device_slot` | mTLS 可省 / Bearer 必填 | 1–128 字符；mTLS 下若出现必须等于服务端解析出的展示 slot（不是证书 CN） |
| `device_id` | 否（服务端注入） | UUID；mTLS 入口落库前由服务端注入 payload，请求体无需携带（见 §0.1） |
| `agent_version` | 是 | 1–64 字符 |
| `collected_at` | 否 | RFC3339；缺省 `now()`，未来 >5 分钟视为非法 |
| `hostname` | 是 | 1–255 字符 |
| `os_name` / `kernel_version` | 否 | 仅展示 |
| `tier` | 否 | `fast` / `full`；缺省 `full`（兼容旧 agent） |
| `health.status` | 是 | `ok` / `warning` / `critical` |
| `health.score` | 否 | 0–100 |
| `health.issues[]` | 否 | `severity` ∈ `info/warning/critical`；`code`/`message` 可读文本 |
| `metrics.*` | 否 | 百分比 0–100；`uptime_secs` ≥ 0 |
| `sections` | 否 | 任意 JSON，原样存档；控制台按已知 key 渲染 |

### 3.1 两档采集语义

| 档位 | 默认周期 | 范围 | 内容 |
|---|---|---|---|
| `fast` | `10s` | `5..=60` | 只跑内置采集（host/CPU/内存/磁盘/网络/温度）；`sections.agent.tools` 五项均为 `{"available": false, "skipped": true}`，`sections` 不含能力采集分区 |
| `full` | `300s` | `60..=3600` | 内置采集 + 安全 qeaudit / 容器 qe-container / 虚机 virt-libvirt（进程内库直调，CLI 兜底）+ platform-health / platform-diagnose（外部命令，各自超时、可选、失败不影响上报） |

- 服务端按 `tier` 落库（`machine_snapshots.tier`，0016）；列表/详情默认展示最近快照，
  快照历史接口可返回两档混合记录。
- **安全扫描独立规则**：qeaudit 按 `ROOM_AGENT_SECURITY_INTERVAL_SECS`（默认 300s，
  0 = 跟随 full 档）独立调度；扫描刷新后**立即**回传一份 full 快照（事件驱动的实时
  回传），服务端按普通 full 快照落库，无新契约字段。
- `--once` 只上报**一份 full**；daemon 分别按两个周期发送（两条独立 deadline，
  采集耗时超周期时顺延，不突发补跑）。
- `--print`（不发请求）在 daemon 下 fast/full 各打印一份契约 JSON。

## 4. 频率配置（服务端下发）

两档间隔属于 `AgentPolicy` 的可下发字段，存储在 `agent_configs`（增量 patch）：

```json
{ "telemetry_fast_interval_secs": 10, "telemetry_full_interval_secs": 300 }
```

- 校验：fast `5..=60`、full `60..=3600`，且 `full >= fast`（合并后校验）。
- 作用域与合并顺序 **global → area → room → device**：
  - `global`：全局默认；
  - `area` / `room`：沿用 0012（`scope_id` UUID）；
  - `device`：机器级覆盖，key 为 `device_slot`（0017 新增 `scope_key`，一个槽位一行）。
- 房间归属：主设备看 `rooms.device_slot`，子机看 `room_devices`；同一槽位不会同时属于两者。

### 4.1 设备拉取：mTLS `GET /agent/config`

- 身份只来自客户端证书（不接受请求体/查询参数覆盖）：
  `serial → device_enrollment(issued/active) → device_slot → (rooms | room_devices) → room+area`；
- 合并 `global → area → room → device`（device 行用解析出的 `device_slot` 匹配 `scope_key`，不是证书 CN）；
- `revision` 取实际应用行的最大值（无行 0）；未绑定房间时只应用 global（+device）；
- 响应：

```json
{
  "revision": 12,
  "policy": {
    "lxc_auto_start": false,
    "poll_interval_secs": 2,
    "download_concurrency": 4,
    "cache_max_bytes": 10737418240,
    "telemetry_fast_interval_secs": 10,
    "telemetry_full_interval_secs": 300
  }
}
```

- room-agent 启动时拉取（失败只 `warn!`，继续用本地 env/默认）；运行期每 30 个
  fast 周期刷新一次。优先级：**远端策略 > 显式 env/CLI > 内置默认**。

### 4.2 管理端接口（用户 JWT + `device:read` / `device:manage`）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/admin/agent-configs` | 所有作用域配置（含 device 行，`scope_key` 为槽位） |
| PUT | `/api/v1/admin/agent-configs` | 写入某作用域 patch；device 作用域用 `device_slot` |
| GET | `/api/v1/admin/agent-configs/effective?room_id=` | 房间合并策略（**不含** device 层，保持旧契约） |
| GET | `/api/v1/admin/agent-configs/effective?device_slot=` | 设备合并策略（global → area → room → device） |

PUT 请求体（device 示例）：

```json
{
  "scope_type": "device",
  "device_slot": "room-101",
  "policy": { "telemetry_fast_interval_secs": 20, "telemetry_full_interval_secs": 900 }
}
```

- `scope_type=global` 不能带 `scope_id` / `device_slot`；`area`/`room` 必须带 `scope_id`；
  `device` 必须带 `device_slot` 且该槽位存在（`device_enrollment` / `rooms.device_slot` /
  `room_devices` 任一命中，否则 404）。
- 写入前会预演「global → area → room → device 合并后的完整策略」，
  不合法（含 `full < fast`）直接 400，不落库。
- 二次下发：写入后对作用域内**每台设备槽位**（房间主设备 + `room_devices` 子机）
  各推一条 `config.update`，payload 为 `{policy, revision}`（用 `effective_for_slot`
  合并，含 device 覆盖）；device 作用域只推该槽位。
- `pushed_rooms`（兼容字段）与 `pushed_slots` 都表示成功入队的设备槽位数。

## 5. 管理端接口（用户 JWT + 权限 `device:read`）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/admin/machines` | 机器分页：设备 + 房间/区域 + 最新快照摘要；支持 `area_id`、`room_id`、`keyword`、`health`、`state`、`page`、`page_size` |
| GET | `/api/v1/admin/machines/summary` | 状态汇总：total/online/stale/offline/missing/alerts（服务端聚合，KPI 不受 page_size 限制） |
| GET | `/api/v1/admin/machines/{serial}` | 机器详情：设备信息 + 最新完整快照 + 最近 N 条历史（`history_limit` 1–50，默认 20） |
| GET | `/api/v1/admin/machines/{serial}/snapshots` | 快照历史元数据（分页，`limit` 1–200） |

`MachineSummary`：

```json
{
  "serial": "4f40ccef...",
  "device_slot": "room-101",
  "status": "active",
  "online": "online",
  "session_online": "online",
  "agent_online": true,
  "report_age_secs": 8,
  "stale_after_secs": 30,
  "report_stale": false,
  "last_seen_at": "2026-10-31T14:52:00Z",
  "room_id": "…", "room_code": "e2e-room", "room_name": "E2E 房间",
  "area_id": "…", "area_code": "e2e-area", "area_name": "E2E 区域",
  "report": {
    "collected_at": "2026-10-31T14:53:01Z",
    "received_at": "2026-10-31T14:53:02Z",
    "agent_version": "0.1.0",
    "hostname": "room-101-node",
    "os_name": "Debian 13",
    "kernel_version": "6.12.0-amd64",
    "health_status": "ok",
    "health_score": 96,
    "cpu_usage_percent": 14.7,
    "memory_usage_percent": 42.1,
    "disk_usage_percent": 61.2,
    "uptime_secs": 3280
  }
}
```

`report` 为 `null` 表示从未上报（控制台显示「未上报」）。

在线字段语义：

| 字段 | 说明 |
|---|---|
| `online` | 综合在线：`device_session.state='online'` 或最新快照未过期。 |
| `session_online` | device-pki 会话原始状态（排障用，可能与 `agent_online` 不一致）。 |
| `agent_online` | Agent 视角严格在线：有快照且 `report_age_secs <= stale_after_secs`。 |
| `report_age_secs` / `stale_after_secs` | 快照年龄与新鲜窗口（秒），用于解释判定。 |
| `report_stale` | `report_age_secs > stale_after_secs`（旧字段名，语义随 v1.4 调整）。 |

### 5.1 实时指标（毫秒级，默认按需）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/admin/machines/{serial}/metrics/stream?interval_ms=` | SSE：`metric`（LiveMetricSample）/ `heartbeat`；`interval_ms` 200–5000，默认 1000 |

- 权限 `device:read`；经 device-pki 内存中转，**不写数据库**。
- 帧事件：`metric`（`LiveMetricSample`）、`op`（`LiveOpProgressEvent`，长操作进度）、
  `command`（`LiveCommandResultEvent`，命令终态，控制台可据此减少 1s 轮询）、`heartbeat`。
- 订阅时按以下顺序通知 agent（新旧兼容）：**机器实时 WS 通道**（v1.5，`want {interval_ms}`，
  最后观察者离开发 `idle`）优先；老 agent 回退 `telemetry.live` 下行 + HTTP `/metrics`
  （TTL 30s，每 20s 续期）。两条路径都不会在无订阅者时产生流量。
- 形状：`{schema:"live-metrics/1", serial, device_slot, collected_at, cpu_usage_percent,
  memory_usage_percent, disk_usage_percent, load1, uptime_secs}`。
- 历史趋势仍由 fast/full 快照提供；实时样本不落库。

## 6. room-agent 配置（客户端侧）

| env / CLI | 默认 | 说明 |
|---|---|---|
| `REPORT_MODE` / `--report-mode` | `mtls` | `mtls`（默认）/ `bearer`（兼容） |
| `DEVICE_PKI_MTLS_URL` | mTLS 必填 | device-pki mTLS 端点（`https://host:4441`） |
| `DEVICE_SLOT` / `--device-slot` | 必填 | 设备槽位 |
| `DEVICE_DATA_DIR` / `--data-dir` | `./data` | 证书在 `<data>/creds/<slot>`（`qe-agent enroll` 写入） |
| `ROOM_AGENT_FAST_INTERVAL_SECS` / `--fast-interval` | `10` | fast 间隔，5..=60 |
| `ROOM_AGENT_FULL_INTERVAL_SECS` / `--full-interval` | `300` | full 间隔，60..=3600 |
| `MACHINE_REPORT_INTERVAL_SECS` / `--interval` | — | 旧参数别名（映射到 fast） |
| `MACHINE_API_BASE` | `http://127.0.0.1:8080/api/v1` | 仅 bearer 模式 |
| `MACHINE_AGENT_TOKEN` | bearer 必填 | 仅 bearer 模式 |
| `ROOM_AGENT_LIVE_PUSH` | 未设置 | `1` = 无条件持续推送实时指标；默认按需（收到 `telemetry.live` 才推） |
| `ROOM_AGENT_LIVE_INTERVAL_MILLIS` | `500` | 本地采样基准间隔（100–5000）；按需推流时由 `interval_ms` 覆盖 |
| `ROOM_AGENT_LIVE_DEMAND` | `1` | 是否接受按需推流；`0` = 关闭按需链路（既不常推也不响应浏览器观测） |
| `ROOM_AGENT_REALTIME_WS` | `1` | 机器实时 WS 通道（`/machine/realtime`）；`0` = 强制回退 HTTP 实时指标 |

`--print` 只打印 payload，不需要证书/token；`--once` 发一份 full 后退出
（成功 0 / 上报失败 1 / 配置错误 2）。

统一入口 `qe-agent run --role machine` 支持同样的两档参数：
`--fast-interval`、`--full-interval`、`--report-mode`、`--interval`（兼容），
数据目录/槽位来自全局 `--data-dir` / `--slot` 或同名环境变量。

## 7. 与既有模块的关系

- `qe-node-agent`（镜像拉取/策略）保持独立；room-agent 负责**机器状态观测**，不参与镜像导入。
- 外部采集依赖是**可选**的：`platform-health` / `platform-diagnose` / `qeaudit` 未安装时
  agent 仍能上报内置指标（/proc、df、docker、virsh），缺失部分在 `sections.agent.tools` 标记；
  fast 档整体跳过外部工具，仅 full 档执行。
- 前端区域/房间总览复用已有接口；机器总览使用本契约新增接口。
- 设备级配置复用 P2 节点配置（`agent_configs`），设备证书由 P0 的 device-pki 注册流程签发。

## 8. 数据库变更

| migration | 内容 |
|---|---|
| `0014_machine_telemetry.sql` | `machine_snapshots`（serial/collected_at 幂等、payload JSONB、保留条数） |
| `0015_room_devices.sql` | 一个房间多台机器：`rooms.device_slot`（主设备）+ `room_devices`（子机） |
| `0016_machine_snapshot_tier.sql` | `machine_snapshots.tier`（fast/full，默认 full） |
| `0017_agent_config_device.sql` | `agent_configs.scope_key` + scope_type 增加 `device`；重建 global 唯一索引、新增 device 唯一索引 |

## 9. 兼容性与迁移

- 旧 room-agent（v1）继续可用：不带 `tier` 的上报按 `full` 处理；Bearer 入口保持；
  `/agent/config` 响应新增字段用默认值填充，旧 node-agent 忽略未知字段。
- 旧客户端未升级时不会应用 device 级频率覆盖（它根本不拉 `/agent/config`）。
- 服务端可先上 0016/0017，再升级客户端；顺序相反也可（0017 之前 device 配置写入会 400）。

## 10. 主机电源事件（v1.6，2026-11-14）

agent 在主机 shutdown/reboot 收尾完成后上报一次「最后一次电源事件」，供管理台回答
「这台机器最后一次是关机还是重启、是否优雅、什么时候」。它与遥测快照相互独立：
即使机器已经断电，事件也已经落库。

### 10.1 上报端点（device-pki mTLS）

`POST /agent/power-event`，身份完全来自设备证书：

- `serial` 取证书 serial；`device_slot` 由服务端按证书身份解析；
- 请求体不能指定其它 serial；
- 返回 `202` + 新事件 id；校验失败 `400`，无证书 serial `401`。

请求体：

```json
{
  "schema": 1,
  "action": "shutdown",
  "graceful": true,
  "requested_by": "agent",
  "agent_version": "0.1.0",
  "at": "2026-11-14T06:00:00Z",
  "note": "VM 2/2 优雅停止；网络已清理（1 个）",
  "details": {
    "trigger": "signal:SIGTERM",
    "hostname": "node-101",
    "vms": [{ "name": "vm-1", "graceful": true, "forced": false }],
    "network": { "ok": true, "networks": ["qevm-101"] },
    "errors": []
  }
}
```

约束：`action` 只接受 `shutdown`/`poweroff`/`reboot`/`restart`（大小写不敏感）；
`details` 必须是 JSON object 且 ≤64 KiB；`note` ≤512 字符。

### 10.2 数据表

`migrations/0043_machine_power_events.sql`：

| 列 | 语义 |
|----|------|
| `serial` | 设备证书 serial（FK `device_enrollment`，级联删除） |
| `device_slot` | 服务端解析出的槽位 |
| `action` | `shutdown` / `reboot` |
| `graceful` | 无 destroy、无清理错误、网络清理成功 |
| `requested_by` / `source` | 发起方 / 来源（当前 `agent`） |
| `agent_version` | 上报 agent 版本 |
| `note` | 一句话摘要 |
| `details` | agent 完整收尾明细 JSONB |
| `created_at` | 服务端时间（排序基准） |

### 10.3 管理端读取

- `GET /api/v1/admin/machines`：每行 `MachineSummary.last_power_event`；
- `GET /api/v1/admin/machines/{serial}`：同上（详情复用摘要字段）；
- 前端机器详情「最近电源事件」展示：时间、动作、是否优雅、发起方、Agent 版本、备注/错误。

`last_power_event` 为 `null` 表示该机器尚未上报过（旧 agent 或从未关机）。

### 10.4 agent 本地兜底

agent 无论上报是否成功，关机前都会先写
`$DEVICE_DATA_DIR/last-shutdown.json`（默认 `/var/lib/qe-agent/last-shutdown.json`）。
上报失败保留 `reported=false`，下次 agent 启动时自动补报；补报失败只记日志，不阻塞启动。
