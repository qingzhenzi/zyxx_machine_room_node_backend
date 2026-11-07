# 网络唤醒（Wake-on-LAN，WOL）

> 痛点 #6 的最小闭环：**agent 上报 MAC → 控制台按机器发起唤醒 → 控制节点在局域网发 magic packet**
> → 目标机网卡在睡眠状态下唤醒主机。本文描述链路、契约、限制与排障。

## 1. 链路

```text
room-agent（目标机，fast/full 档都会采集）
  └─ /sys/class/net/*/address ──► machine.report ──► sections.agent.host.macs
                                                        │
                         qe-api / PostgreSQL（machine_snapshots.payload 原样存档）
                                                        │
控制台「机器详情 → 网络唤醒」按钮                        │
  └─ POST /api/v1/admin/machines/{serial}/wake（device:wake）
        └─ 读取最新快照 sections.agent.host.macs
             └─ UDP 广播 255.255.255.255:9，每个 MAC 发 3 次
                  magic packet = 6×0xFF + MAC×16（102 字节）
                        └─ 目标机网卡（BIOS/UEFI 已开 WoL）唤醒主机
```

- **采集端**：`room_agent/crates/qe-agent-telemetry` 的内置 host 采集读取
  `/sys/class/net/<iface>/address`，跳过回环/全零，转小写冒号格式、排序去重，
  落在 `sections.agent.host.macs`（契约见 [`machine-telemetry.md`](machine-telemetry.md) v1.3 §0.2）。
- **控制端**：`qe-api` 从该 serial **最新一条** `machine_snapshots.payload` 取 MAC；
  magic packet 构造是纯函数（`qe_domain::wol::magic_packet`），API handler 只负责 UDP 发送与审计。
- **目标机**：magic packet 由网卡在 S3/S4/S5 下监听（需 BIOS/UEFI 与网卡驱动开启 WoL）。

## 2. 前置条件与硬限制

| 条件 | 说明 |
| --- | --- |
| **同一二层网络（L2）** | magic packet 是链路层广播，**不能跨网段/跨 VLAN 路由**。控制节点（qe-api 所在机器/容器网络）必须与目标机在同一广播域；若控制节点跑在 Docker bridge 里，需要 `network_mode: host` 或把广播包从宿主机发出。 |
| **目标机 BIOS/UEFI 开启 WoL** | 常见名称：Wake on LAN / Power On By PCI-E / Resume by LAN。关闭时网卡不会在 S5 供电。 |
| **网卡/驱动支持并启用** | Linux：`ethtool <iface>` 里 `Supports Wake-on: g` 且 `Wake-on: g`；`ethtool -s <iface> wol g` 持久化需 systemd/udev 或 `networkd` 配置。 |
| **睡眠模式** | S3（挂起到内存）/S4（休眠）/S5（软关机）可唤醒；**S5 需要主板 5VSB 待机供电 + BIOS 开关**；笔记本电池模式/节能设置可能禁用。 |
| **交换机/防火墙** | 不需要开放端口 9 的入站规则（UDP 广播），但交换机需允许广播（部分交换机的广播风暴抑制/端口隔离会丢弃）；VLAN 隔离会阻断。 |
| **agent 曾有快照** | MAC 来自最新快照；机器换网卡/从未上报过 → 没有可用 MAC（API 返回 422），需先让 agent 上报一次。 |

## 3. 权限与接口

- 权限点：**`device:wake`**（迁移 `migrations/0030_wol_permission.sql` 幂等播种，授予 `admin` 角色；
  `operator` 等业务角色由管理员在「角色权限」页按需勾选）。
- 接口：`POST /api/v1/admin/machines/{serial}/wake`（管理端 JWT）。

成功响应（HTTP 200）：

```json
{
  "ok": true,
  "serial": "4f40ccef...",
  "macs": ["aa:bb:cc:dd:ee:01"],
  "sent_to": ["aa:bb:cc:dd:ee:01"],
  "warnings": []
}
```

- `ok`：至少有一个 magic packet 成功写入 UDP socket（不保证目标机已醒）；
- `sent_to`：至少成功发出一次的 MAC；
- `warnings`：发送失败的 MAC/原因；**发送失败不改变 HTTP 语义**（仍是 200），由控制台 toast 展示。

错误语义（与其它 admin 接口一致）：

| 状态 | 场景 |
| --- | --- |
| 401 | 未登录 / token 失效 |
| 403 | 缺少 `device:wake` |
| 404 | serial 对应的设备不存在 |
| 422 | 机器存在但**从未上报快照**，或最新快照的 `sections.agent.host.macs` 为空 |
| 503 | 数据库暂不可用 |

审计：每次调用写一条 `audit_logs`，`action=device.wake`、`resource_type=machine`、
`resource_id=<serial>`，`detail` 含 `macs` / `sent_to` / `warnings` / `snapshot_id`。

## 4. 控制台

「机器总览 → 机器详情」右上角「网络唤醒」按钮：

- 无 `device:wake` 权限时不显示；
- 最新快照没有 MAC 时按钮禁用，hover 提示原因；
- 点击后弹确认框（列出目标 MAC 与限制），确认后调用 API；
- 成功 toast 列出 MAC；`ok=false` 或接口 4xx/5xx 用错误 toast（422/404 会显示后端 message）。

## 5. 失败排查

| 现象 | 排查 |
| --- | --- |
| 控制台按钮灰色 / 接口 422 | 机器最新快照没有 `sections.agent.host.macs`：确认 agent 已升级到含 MAC 采集的版本、`/sys/class/net` 可读（容器内需要 host 网络/挂载）、机器有非回环网卡。 |
| `warnings` 非空 / `ok=false` | 看 warning 原因：`Permission denied`（容器无 NET_RAW/广播权限）、`Network is unreachable`（无默认路由）、`set_broadcast` 失败。 |
| 发送成功但机器没醒 | 依次确认：目标机 BIOS WoL 是否开启；`ethtool <iface>` 的 `Wake-on: g`；是否 S5 且主板待机供电；交换机广播/VLAN；控制节点是否在**同一广播域**（Docker bridge 默认不是）。 |
| 机器换了网卡后失效 | 旧快照仍有旧 MAC；等 agent 下一次上报（fast 10s）刷新，或重启 agent 立即上报。 |
| 只能唤醒部分机器 | 不同房间/网段需要各自网段内的控制节点发广播；跨网段路由无法转发广播，需在目标网段部署转发器（如 `wol-relay`）。 |

## 6. 本机冒烟记录（2026-11-07）

- 环境：控制节点与“目标机”同机（127.0.0.1 所在 LAN），无真实待机目标机；
- 方式：向 dev 库插入一条含 `sections.agent.host.macs` 的临时快照 → 登录 admin →
  `POST /api/v1/admin/machines/{serial}/wake` → 记录响应 → 删除临时快照；
- 结果：见任务报告（本机 UDP 广播发出后的 `ok/sent_to/warnings`）；“目标机是否真的醒来”
  无法在本机验证（无待机机器），属于现场验收项。

## 7. 后续可做（不在最小闭环内）

- 按房间/区域批量唤醒（一个按钮发多台）；
- 记录唤醒历史与结果（目前只在审计日志里）；
- 控制节点与目标机不同网段时的 WOL relay 部署方案；
- 唤醒后自动等待 agent 上线并在控制台提示（可复用机器 SSE）。
