# 机器实时观测：现状、问题与改进方案

> 2026-11-14。对应需求：**高确定机器在线/状态**、**状态变更要有单独事件**、
> **实时指标默认较实时（任务管理器式）**、**支持更高刷新率**。
>
> 本文先给结论，再列现状证据、已落地改动（P0 切片）、后续阶段与风险。
> 契约字段以 `docs/machine-telemetry.md` 为准；在线事件见 `docs/storage-design.md` §18.5。

## 0. 结论（TL;DR）

1. **在线判定不能再写死 5 分钟**。快照新鲜窗口改为 `max(3×agent fast 间隔, 30s)`，
   由 agent 在 payload 里自报的 `sections.agent.fast_interval_secs` 推导；接口同时返回
   `report_age_secs` / `stale_after_secs` / `agent_online`，前端能解释「为什么在线」。
   **已落地**。
2. **只跑 machine 角色（不拉下行）的设备没有 `device_session` 行**，管理端 `session_online`
   永远是 null、事件也永远不产生。`/machine/report` 现在同时节流落库会话（15s 节流）。
   **已落地**。
3. **恢复在线默认不再静默**：`DEVICE_PRESENCE_NOTIFY_ONLINE` 默认改为 `true`，
   `device.online` / `device.offline` 都进入事件流；机器列表页直接订阅
   `/admin/device-events/stream`，在线/离线即时改行 + toast + 行内标记，而不是每 5s 整页重载。
   **已落地（前端列表）**。
4. **实时指标从「默认关、开了就常推」改为「默认开、按需推、可调刷新率」**：
   浏览器打开实时流时 qe-api 通知 agent 按 `interval_ms`（200–5000ms）推流；
   页面关闭后立即/最多 30s 停止，无订阅者时 agent 不产生毫秒级流量。
   详情页默认开启实时指标，可选 200ms/500ms/1s/2s/5s。**已落地**。
4.1 **agent→device-pki 改为一条 mTLS WS 长连接（v1.5）**：`/machine/realtime`
   同时承载实时样本与长操作进度，服务端用 `want/idle` 直接控制频率与开关；
   连不上自动回退 HTTP。关机等长操作的 `accepted/cleanup_done/scheduled` 阶段经
   SSE `op` 事件到控制台。**已落地**。
5. **仍建议后续做（未落地）**：设备会话拆分 `last_seen_at` / `state_changed_at` 列、
   `machine.state` 事件帧（把状态与快照放同一条 SSE）、在线检测窗口从 ~15–20s 收紧到
   ~10s、列表状态筛选与时间线。见 §5。

## 1. 旧方案哪里不好（问题清单）

| # | 问题 | 影响 |
|---|---|---|
| 1 | `online = 会话 online` **或** `快照 < 5 分钟` | 机器已死仍显示在线最多 5 分钟；只上报的设备 5 分钟后变 `null`；两个事实混在一个字段里，无法解释 |
| 2 | `report_stale` 固定 5 分钟 | 与 fast=10s 的采集节奏脱节，观测上不可信 |
| 3 | 只跑 machine 角色的设备不写 `device_session` | `session_online` 恒 null、presence 事件不产生、工单离线规则失效 |
| 4 | 设备恢复在线默认不发事件 | 状态变化只看到「掉线」，看不到「回来了」 |
| 5 | 列表页每 5s 全量刷新 + 每条快照 SSE 触发全量 reload | 行不原地更新、状态变化不明显、放大 DB/网络压力 |
| 6 | 实时指标默认关；开启后 500ms 常推、无订阅者也推 | 默认体验是 10s 快照（甚至详情页冻结）；开启后全 fleet 常驻流量 |
| 7 | 无刷新率控制；「实时」绿灯只代表 SSE 开着 | 没有数据也显示实时，误导运维 |
| 8 | 在线/离线没有页面内事件提示（只有通知铃铛 30s 轮询） | 「机器状态发生变更也不是很明显」 |

## 2. 统一状态模型（接口契约）

`MachineSummary` 现返回（新增字段向后兼容）：

| 字段 | 含义 | 计算 |
|---|---|---|
| `online` | 综合在线：会话 online 或快照未过期 | `ds.state='online' OR report_age <= stale_after` |
| `session_online` | device-pki 会话原始状态（排障用） | `device_session.state` |
| `agent_online` | **Agent 视角严格在线**：有快照且未过期（不并集会话） | `report_age <= stale_after` |
| `report_age_secs` | 最新快照年龄（秒） | `now - collected_at` |
| `stale_after_secs` | 新鲜窗口 | `max(3 × sections.agent.fast_interval_secs, 30)`，读不到按 10s 兜底 |
| `report_stale` | 是否过期（沿用旧字段名） | `report_age > stale_after` |
| `last_seen_at` | 最近心跳或快照接收时间 | `GREATEST(ds.updated_at, ms.received_at)` |

**判定口诀**：会话回答「传输层活着吗」，快照回答「agent 还在采吗」；页面主状态用
`agent_online`（观测口径），`session_online` 作为副标签展示。这样「机器明明关了但显示在线
最多 5 分钟」会被压到「窗口（默认 30s）+ 采集间隔」内。

**事件模型（现状）**：`device_presence_events` 只记录 `online/offline` 翻转，SSE
`/admin/device-events/stream` 每 2s 查库增量；默认离线必发、在线现在默认也发。
非房间绑定设备不产生事件（presence 扫描 `JOIN rooms`），这是后续要补的 P1 项。

## 3. 实时指标链路（按需推流）

```
浏览器详情页
  │ GET /admin/machines/{serial}/metrics/stream?interval_ms=1000   (SSE)
  ▼
qe-api ──(device_downlink)──► room-agent：telemetry.live
  │   {enabled:true, interval_ms:1000, expires_at_ms: now+30s}       │
  │   （SSE 存活期间每 20s 续期；断开后不再续期）                     ▼
  │                                                    <data_dir>/telemetry.live.json
  │                                                                  │
  │                                          qe-agent-telemetry 采样线程按 interval_ms
  │                                                                  │
  ▼                                                       POST /metrics (mTLS, 不写 DB)
device-pki MetricsHub（内存广播） ◄──────────────────────────────────┘
  │ GET /internal/metrics/{serial} (内部 WS)
  ▼
qe-api SSE ──► 浏览器（默认 1s 渲染，可 200ms–5s）
```

要点：

- **默认按需**：`ROOM_AGENT_LIVE_PUSH=1` 仍保留「持续推流」旧行为；否则只有收到
  `telemetry.live` 且未过期才推。无浏览器观测时 agent 不产生毫秒级流量。
- **更高刷新率**：`interval_ms` 200–5000 由浏览器选择，agent 采样/推送同步变快；
  超出范围的请求 400。
- **TTL 自愈**：agent 侧按 `expires_at_ms` 自动停止；qe-api 崩溃/重启只会让推流最多
  多跑 30s，不会永久泄漏。
- **老 agent 兼容**：不认 `telemetry.live` 的 agent 会 ack 跳过（dispatcher 对未知 kind
  按已知悉处理），行为退回旧版（仅 `ROOM_AGENT_LIVE_PUSH=1` 时推流）。
- **不落库**：实时样本只走内存广播；历史仍由 fast/full 快照提供。需要更长窗口时应做
  降采样/ring buffer（P2），而不是逐样本入库。

### 3.1 实时 WS 通道（v1.5，agent ↔ device-pki）

```text
agent ──mTLS WSS /machine/realtime──► device-pki
  {"type":"hello", agent_version, capabilities}
  {"type":"sample","payload":{...}}        // 仅收到 want 后按 interval_ms 发
  {"type":"op_progress", request_id, kind, stage, percent, message}
  ◄── {"type":"ready"} / {"type":"want","interval_ms":1000} / {"type":"idle"}
```

为什么用它替代逐样本 HTTP POST：

| 维度 | HTTP POST /metrics（v1.4） | 实时 WS（v1.5） |
|---|---|---|
| 连接开销 | 每样本一次请求（200ms 档 ≈5 req/s/机器） | 一条长连接，帧级推送 |
| 频率控制 | 靠 `telemetry.live` 下行 + 文件/TTL 间接控制 | 服务端 `want/idle` 直接控制 |
| 断网 | 采样线程被同步 POST 阻塞（最长 10s/次） | 写入 WS 缓冲，失败即重连；采样不受单次请求阻塞 |
| 操作进度 | 只能轮询命令结果 | 同一条连接推送 `op_progress`，控制台实时显示阶段 |
| 兼容 | — | 连不上自动回退 HTTP；`ROOM_AGENT_REALTIME_WS=0` 可强制回退 |

边界：WS 只承载**可丢的实时流**与进度；命令下发/结果回传仍走既有
`device_downlink` + `machine_command_results`（至少一次、可重放），不用 WS 替代可靠通道。

## 4. 控制台交互（默认「较实时」）

- **机器列表**：SSE 快照 + presence 事件；快照原地 patch 行，在线/离线事件即时改行并
  toast（同 serial 5s 去重、首屏 3s 静默）；行内显示「刚刚离线/刚刚上线」60s；刷新节奏
  可选「实时（默认）/5s/15s/30s/暂停」，选择持久化到 localStorage；后台仅在非「实时」
  或兜底时全量对账。
- **机器详情**：实时指标**默认开启**（记住用户选择），刷新率 200ms–5s；绿灯只在
  「3×刷新率内收到样本」时亮，否则「等待数据」；`自动刷新` 默认开。关机命令
  202 后轮询 `GET /agent/commands/{request_id}` 作为兜底，同时实时通道的 SSE `op`
  事件会推 `accepted/cleanup_done/scheduled` 阶段，详情页直接显示
  「关机中 · 已清理虚拟机/容器」→「agent 已接收，正在关机」→ 机器离线；
  离线机器下发时明确提示「命令会排队」。
- **通知/事件页**：`device.online/offline` 均可见；详情/列表状态变化不再依赖 30s 轮询。
- **KPI 与筛选**：列表 KPI 改用服务端 `GET /admin/machines/summary` 聚合（不再只数
  page_size=100）；新增 `state=online/stale/offline/missing` 观测状态筛选。
- **命令结果事件**：device-pki 把 `machine_command_results` 终态通过实时广播发出，
  qe-api 以 SSE `command` 事件转发；详情页优先用事件、轮询作兜底。

## 5. 后续阶段

### P1（建议下一轮）
1. `device_session` 拆列：`last_seen_at`（活动时间，始终前进）与 `state_changed_at`
   （翻转时间）；事件 `last_seen_at` 不再语义混淆。
2. 离线检测窗口收紧：`DEVICE_OFFLINE_TICK_MS` / `HEARTBEAT_TIMEOUT_MS` 可配，
   目标 ~10s 判离线；活动写库节流 15s→5s（先做压测）。
3. `machine.state` SSE 帧：列表 SSE 同连接下发 `{serial,state,state_reason,state_changed_at}`，
   页面无需再单独订阅设备事件（仍可复用）。
4. presence 事件不再要求房间绑定（LEFT JOIN，房间/区域可空）；qe-api 重启时从 DB
   种子化 tracker，避免停机期间漏事件。
5. 状态筛选/排序（离线、降级、最近在线）；KPI 改用服务端聚合（当前按 page_size=100 统计）。
6. `sections.live_metrics` 去重（只提交增量或去掉 `items`），避免每 10s 重复 ~35KB。

### P1 进展（2026-11-14）
- ✅ agent→device-pki 实时 WS 通道 + `want/idle` 按需限速（替代逐样本 POST）。
- ✅ 长操作进度 `op_progress` → SSE `op` → 控制台“执行中”（关机阶段已接）。
- ✅ 命令终态 `command_result` → SSE `command` → 详情页事件优先、轮询兜底。
- ✅ presence 覆盖未绑定房间设备（LEFT JOIN，房间/区域可空），qe-api 重启从事件表
  补基线，停机期间的翻转不会被永久漏掉。
- ✅ KPI 改服务端聚合（`/machines/summary`），列表新增 `state` 筛选。
- ✅ `sections.live_metrics` 去掉重复的 600 点原始样本，只保留聚合。
- 下一步：`device_session` 拆列（last_seen_at / state_changed_at）；离线检测窗口
  收紧到 ~10s；命令下发也考虑走同一条 WS（保留可靠通道语义）。

### P2
- 统一 WS（状态 + 实时指标 + 命令结果）替代多条 SSE + 轮询；
- 实时样本降采样/短窗口持久化（1s→1min rollup）；
- 状态变更审计/告警规则（silent machine、抖动设备最小驻留时间）；
- 1000+ 台压测，SSE 查库轮询改 LISTEN/NOTIFY 或 Redis Streams。

## 6. 风险与权衡

| 风险 | 说明 / 缓解 |
|---|---|
| `online` 语义变化 | 保留旧字段，新增 `agent_online` / 年龄字段；页面主状态用新字段，旧消费方不受影响 |
| 在线事件变多 | `device.online` 默认开启后恢复在线也发事件；presence 仍有 250ms 去抖与唯一键去重；抖动设备需 P1 加最小驻留 |
| 按需推流依赖 agent 升级 | 老 agent 忽略下行，行为退回旧版；控制台显示「等待数据」而不是假装实时 |
| 多副本 device-pki | 实时指标 demand 是进程内状态，多副本需粘性路由或 Redis（P2） |
| 无图形会话机器 | 运行期关机提示只在有图形会话 + 显式 `QE_BOOT_DISPLAY_BIN` 时拉起；重启预算 5 次，避免 crash-loop |
| 系统关机拆画面 | 提示在 cleanup 前推送，`QE_AGENT_POWEROFF_DELAY_SECS` 默认 3s；最后 1–3s 可能因合成器退出而定帧 |

## 7. 验收清单

- [ ] 只跑 `--role machine` 的机器：kill 后 ≤ `stale_after + 事件 tick` 显示离线，恢复后出现 `device.online`
- [ ] 列表页不因快照 SSE 整页 reload；在线/离线改行 + toast，60s 内行有标记
- [ ] 详情页默认实时指标；关闭页面后 `journalctl -u qe-agent` 30s 内停止 `/metrics` 推送
- [ ] 切换 200ms/1s/5s 后实时样本间隔随之变化，绿灯只在有样本时亮
- [ ] 控制台关机：机器屏幕出现「正在关机」→「N 秒后执行」；控制台显示「已下发 → agent 已接收」；机器随后离线
- [ ] 无图形会话/禁用展示层的机器：关机命令照常执行，`boot-display` 不 crash-loop
