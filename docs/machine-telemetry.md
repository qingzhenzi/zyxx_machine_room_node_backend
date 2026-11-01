# 机器遥测契约（room-agent → device-pki / qe-api）

> 目标：房间内的「子机器端」周期性采集主机清单 + 健康结论 + 诊断结果，上报到服务端，
> 管理控制台在「机器总览」里看到每台机器的实时状态与历史快照。
>
> 本文是 **v1.1 契约**；`room-agent` 实现、`qe-device-pki` 的 mTLS handler 与
> `qe-api` 的管理端接口都以本文为准。相对 v1 的变化见 §0。

## 0. v1.1 相对 v1 的变化

| 变化 | v1 | v1.1 |
|---|---|---|
| 默认认证 | fleet Bearer token（`MACHINE_AGENT_TOKEN`） | **设备证书 mTLS**（`POST {DEVICE_PKI_MTLS_URL}/machine/report`）；`REPORT_MODE=bearer` 保留旧路径 |
| 采集频率 | 固定一档（默认 15s，10..=3600） | **两档**：fast（内置指标，默认 10s，5..=60）与 full（含外部工具，默认 300s，60..=3600） |
| payload `tier` | 无 | `"fast"` / `"full"`（`machine_snapshots.tier`，0016） |
| 频率配置 | 只能改本地 env | 服务端 `agent_configs` 下发：**global → area → room → device** 合并（0017） |
| 契约版本 | `schema: 1` | 仍为 `schema: 1`（新增字段向后兼容；旧 agent 不带 `tier` 按 full 处理） |

## 1. 角色与链路

```text
room-agent（每个房间机器）
   ├─ fast 档（默认 10s）：内置 /proc、/sys、df、hwmon —— 不启动外部工具
   ├─ full 档（默认 300s）：内置采集 + platform-health / platform-diagnose / qeaudit / docker / virsh
   ├─ 启动/运行期：mTLS GET {DEVICE_PKI_MTLS_URL}/agent/config 拉两档间隔
   └─ mTLS POST {DEVICE_PKI_MTLS_URL}/machine/report        ──► qe-device-pki ──► PostgreSQL
                                                                            │
                        （兼容路径）Bearer POST /api/v1/machine/report ──► qe-api ──┘
                                                                            │
                       管理控制台 /machines ◄── /api/v1/admin/machines ─────┘
```

- **默认（mTLS）**：身份来自设备证书（`device-pki` 从证书 CN/serial 解析
  `device_slot`/`serial`），请求体不需要 `Authorization`；`device_slot` 可省略，
  若带上则必须与证书 CN 一致。
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
  "device_slot": "room-101",
  "received_at": "2026-10-31T14:53:02Z"
}
```

- 幂等：同一 `(serial, collected_at)` 重复上报只保留一条。
- 错误：`400` 校验失败 / 槽位与证书不一致；`401` 证书无存活注册记录；`500` 内部错误。
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
      "host": { "machine_id": "..." }
    }
  }
}
```

字段约束：

| 字段 | 必填 | 说明 |
|---|---|---|
| `schema` | 是 | 固定 `1`（未知版本 400） |
| `device_slot` | mTLS 可省 / Bearer 必填 | 1–128 字符；mTLS 下若出现必须等于证书 CN |
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
| `fast` | `10s` | `5..=60` | 只跑内置采集（host/CPU/内存/磁盘/网络/温度）；`sections.agent.tools` 五项均为 `{"available": false, "skipped": true}`，`sections` 不含外部工具分区 |
| `full` | `300s` | `60..=3600` | 内置采集 + platform-health / platform-diagnose / qeaudit / docker / virsh（各自超时、可选、失败不影响上报） |

- 服务端按 `tier` 落库（`machine_snapshots.tier`，0016）；列表/详情默认展示最近快照，
  快照历史接口可返回两档混合记录。
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
- 合并 `global → area → room → device`（device 行用证书 CN 匹配 `scope_key`）；
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
| GET | `/api/v1/admin/machines` | 机器分页：设备 + 房间/区域 + 最新快照摘要；支持 `area_id`、`room_id`、`keyword`、`health`、`page`、`page_size` |
| GET | `/api/v1/admin/machines/{serial}` | 机器详情：设备信息 + 最新完整快照 + 最近 N 条历史（`history_limit` 1–50，默认 20） |
| GET | `/api/v1/admin/machines/{serial}/snapshots` | 快照历史元数据（分页，`limit` 1–200） |

`MachineSummary`：

```json
{
  "serial": "4f40ccef...",
  "device_slot": "room-101",
  "status": "active",
  "online": "online",
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

`report` 为 `null` 表示从未上报（控制台显示「未上报」）。`online` 来自 `device_session`。

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
