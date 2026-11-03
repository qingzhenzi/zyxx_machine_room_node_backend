# 工单（Ticket）系统后端现状清单与 P0 方案

> 调研范围：`crates/`（api / domain / db / auth / cache / device-pki）、`migrations/`、`docs/`、`README.md`。
> 结论均为只读调研，**本文件之外未修改任何源码**。引用格式 `path:line`（行号以当前工作区为准）。
> 结论先行：**需要工单，但不需要“再建一套告警系统”**。仓库里已有的信号（机器健康、设备离线、投递失败、分发失败、镜像校验失败）全部是“只记录、不闭环”，工单层的价值就是补上 **assignee / status / 处理轨迹 / 幂等去重** 这四件事。

---

## 0. 现状：仓库中不存在任何工单概念（已核实）

- 全部 Rust 源码模块清单里没有 ticket 相关模块：`crates/db/src/lib.rs:6-20`、`crates/api/src/api/admin/mod.rs:1-15`、`crates/api/src/api/mod.rs:1-8`。
- 全部迁移 `0001`–`0017` 中没有 ticket/工单表：`ls migrations/`（最后两个是 `0016_machine_snapshot_tier.sql`、`0017_agent_config_device.sql`）。
- 权限点共 **21 个**，无 ticket 权限：`crates/domain/src/rbac.rs:32-54`，其中 `ALL.len()` 断言在 `crates/domain/src/rbac.rs:95`。
- `README.md` 的功能目录（`README.md:5-16`）与后台菜单均无工单；全仓 `ticket|工单|work_order` 关键字检索为 0 命中。
- 现有“后台可操作面”只有 CRUD 类管理（区域/房间/课表/设备/存储/镜像/用户/角色/公告/审计），没有任何“某人负责某问题直到关闭”的实体。

因此工单是**净新增模块**，不需要迁移任何旧业务数据；唯一的改造点是“信号源 → 工单”的生成器与 RBAC/路由/OpenAPI 的登记。

---

## 1. 应接入工单的现有信号源（逐项：表/列、语义、阈值、API+权限、为何不可行动）

### 1.1 总览矩阵

| # | 信号源 | 事实表 | 触发语义 | 现成阈值/字段 | 现有查询 API + 权限 | 为什么今天不可行动 |
|---|---|---|---|---|---|---|
| S1 | 机器健康 | `machine_snapshots` | room-agent 周期上报的健康结论 ok/warning/critical | `health_status`、`health_score`、`payload.health.issues[]`、`tier`(fast 10s/full 300s) | `GET /admin/machines`、`/machines/{serial}`、`/machines/{serial}/snapshots`、`/machines/stream`，全部 `device:read`（`crates/api/src/api/admin/machines.rs:101,123,156,182`） | 只读看板；SSE 只推快照（`machines.rs:290-297`），无“问题”实体、无负责人、无处理状态 |
| S2 | 设备离线 | `device_presence_events` + `device_presence_deliveries` | `device_session` 翻转产生 offline/online 事件 | `previous_state/current_state`、`source_updated_at`、`occurred_at`、webhook 投递 `failed` | `GET /admin/device-events`、`/stream`、`/admin/device-presence-deliveries`，全部 `device:read`（`crates/api/src/api/admin/device_events.rs:122,137,165`） | 事件表是“发生过什么”的流水（`crates/api/src/presence.rs:15`），投递失败终态无重试接口、无认领 |
| S3 | 课表/事件投递失败 | `schedule_events` + `schedule_event_deliveries` | 触发规则命中的 webhook/pubsub/downlink 投递 | `deliveries.status IN(pending,delivered,failed,skipped)`、`attempts`、`last_error`、`next_attempt_at` | `GET /admin/schedule-events`、`/{id}`、`/{id}/deliveries/{did}/retry`，权限 `schedule:event:read` / `schedule:event:manage`（`crates/api/src/api/admin/schedule_events.rs:122,168,190,218`） | 只能人工翻列表/点重试；`failed` 是终态，没有任何“待处理”提示 |
| S4 | 存储分发/收集 | `transfers`、`collections` | 镜像分配创建 download 任务；节点回传收集 | `transfers.status IN(pending,running,completed,failed,expired)`、`progress`、`error`（`crates/domain/src/storage.rs:227-266`）；`collections.status IN(pending,uploaded,verified,rejected)`（`storage.rs:267-300`） | `GET /admin/transfers`、`POST /admin/transfers/{id}/retry`、`GET /admin/collections`，`storage:read/manage`（`crates/api/src/api/admin/storage.rs:262,281,330`） | `failed/expired` 只能手动重试；`collections.rejected` 无任何后续流程；没有“这次失败谁跟” |
| S5 | 镜像验证/发布/分配 | `image_verifications`、`image_assignments` | verify 逐项检查（sha256/size/magic/签名），失败退回 `manifest_ready` | `ok`、`report.checks[]`、`verified_at`；分配 `UNIQUE(room_id,image_id)` | `POST /admin/images/{id}/verify`（`images.rs:611`，`image:manage`），`GET /admin/images/{id}`（`image:read`） | 失败只写 `image_verifications` + 审计 `image.verify`（`images.rs:785-800`），没有人会收到“这个镜像坏了一直躺着”的提示 |
| S6 | 节点配置下发 | `agent_configs` + `device_downlink` | 写配置后向受影响槽位推 `config.update` | `pushed_slots` 只作为响应统计；`device_downlink.delivered_at IS NULL` 表示未 ACK | `PUT /admin/agent-configs`（`device:manage`，`agent_configs.rs:146`）、`GET /admin/agent-configs/effective`（`device:read`） | 下发失败只 `warn!` 日志（`crates/api/src/api/admin/agent_configs.rs:296-321`、`dispatch_config_update` 内部），**没有持久失败信号**，无法生成工单（需 P1 补） |
| S7 | 审计/安全 | `audit_logs` | 管理写操作审计；登录失败计数只在 Redis | `action`、`resource_type`、`resource_id`、`detail`、`ip`、`created_at` | `GET /admin/audit-logs`，`audit:read`（`crates/api/src/api/admin/audit.rs:29`） | 审计是“事后追责”，不是“待办”；登录锁定在 Redis 带 TTL（`crates/cache/src/cache.rs:148-191`），过期即消失，不可复盘 |

### 1.2 S1 机器健康（最核心的工单来源）

- 表结构：`migrations/0014_machine_telemetry.sql:9-37`。关键列：`serial`/`device_slot`、`collected_at`、`received_at`、`health_status TEXT CHECK IN ('ok','warning','critical')`、`health_score INT 0..100`、`payload JSONB`（完整上报原样存档）、唯一约束 `(serial, collected_at)`（`0014:30`，多副本/重传幂等）。
- 分档：`migrations/0016_machine_snapshot_tier.sql:8-15` 增加 `tier IN ('fast','full')`。fast 默认 10s、只跑内置指标；full 默认 300s、含外部工具（`docs/machine-telemetry.md:133-139`）。**这是告警噪音的主要来源**（10s 级抖动）。
- 问题明细：`health.issues[]` 的 `severity ∈ info/warning/critical`、`code`、`message`（`docs/machine-telemetry.md:82-90,126-130`；结构体见 `crates/domain/src/machine.rs:52-60`）。注意 `issues` 仅存于 `payload` JSONB（`0014:25`），没有列级抽取 —— 工单 worker 需要 `payload->'health'->'issues'`，这是 P0 可接受的（单 serial 最新一行，成本低）。
- 写入/裁剪：`crates/db/src/machine.rs:287-352`（`ON CONFLICT (serial, collected_at) DO NOTHING` + 每 serial 保留 `MACHINE_SNAPSHOT_KEEP` 条，默认 100）。
- 查询：`crates/db/src/machine.rs:354-...` 列表 JOIN `device_session`/`rooms`/`room_devices`/`areas`；`health=missing` 表示从未上报（`machine.rs:128-129,234-235`）。接口 `crates/api/src/api/admin/machines.rs:96-171`。
- **不可行动的原因**：接口只返回最新快照与历史元数据，没有“待处理问题”的聚合；SSE 只广播 `machine.snapshot` 元数据帧（`machines.rs:290-297`），订阅者收到后仍需自己判断严重性；没有任何人/状态字段。

### 1.3 S2 设备离线事件

- 事件表：`migrations/0013_device_presence.sql:12-25`，唯一约束 `(serial, current_state, source_updated_at)` 保证多副本同一次翻转只落一行；投递表 `0013:27-42`，唯一 `(event_id,target_url)`，worker 用 `FOR UPDATE SKIP LOCKED` 领取（`crates/db/src/presence.rs:123-147`）。
- 语义：只在**状态翻转**时产生事件，首轮只建基线不重放（`crates/api/src/presence.rs:86-116`）；状态事实来源仍是 `device_session`（`0013` 注释第 1 行；`0006_device_pki.sql:43-49`）。设备被判 offline 由 device-pki 的“连续 miss 阈值”决定（`crates/device-pki/src/session.rs:164-185`，README 说明 `README.md:851-887`）。
- 阈值字段：`occurred_at`、`source_updated_at`、`last_seen_at`（在 payload 里，`presence.rs:63-84`）。**没有**“离线多久算故障”的字段/配置，也没有“预期在线”标记。
- 查询/权限：`device-events` 三接口 + SSE，`device:read`（`device_events.rs:117-...`、`admin/mod.rs:143-149`）。
- **不可行动的原因**：事件是流水而非待办；投递失败没有重试端点（对比课表投递有 `replay/retry`，`admin/mod.rs:102-111`）；离线可能只是教室正常关机/断电，需要业务判定，当前没有任何抑制/确认机制。

### 1.4 S3 课表事件投递失败

- 表：`migrations/0009_schedule.sql:165-181`（`schedule_events.event_key UNIQUE` 幂等）、`184-198`（`schedule_event_deliveries`，`status IN(pending,delivered,failed,skipped)`，`UNIQUE(event_id,target)`）。
- 语义/状态机：`crates/api/src/scheduler.rs:1-17`（先落库、每目标一行、失败指数退避、永久不可达置 skipped）；重试上限 `SCHEDULE_WEBHOOK_MAX_ATTEMPTS`，默认 5（`crates/api/src/config.rs` 对应字段）。
- 失败字段：`last_error`、`attempts`、`next_attempt_at`、`delivered_at`；注意该表**没有 `updated_at`**（`0009:184-198`），按时间聚合要 JOIN `schedule_events.triggered_at`。
- API/权限：`schedule:event:read` 列表/详情、`schedule:event:manage` replay/retry（`schedule_events.rs:122,168,190,218`），写入审计 `schedule_event.replay/retry`（`schedule_events.rs:197,228`）。
- **不可行动的原因**：`failed` 是终态后静默；没有“待处理”聚合、没有负责人；聚合维度（房间×目标×日期）需要新查询。

### 1.5 S4 存储分发/收集

- 表：`migrations/0011_storage.sql:86-98`（`transfers`：direction/room_id/image_id/manifest_digest/status/progress/error/updated_at）、`101-111`（`collections`）。
- 状态机：`crates/domain/src/storage.rs:223-266`（pending→running→completed/failed/expired）、`crates/domain/src/storage.rs:267-300`（pending→uploaded→verified/rejected）。`device-pki` 侧负责推进：`crates/device-pki/src/transfer.rs:110-140`（running / 终态）。
- API/权限：`GET /admin/transfers`、`POST /admin/transfers/{id}/retry`（仅 failed/expired 可重试，`crates/api/src/api/admin/storage.rs:275-309`）、`GET /admin/collections`；`storage:read/manage`。
- **不可行动的原因**：retry 是人工动作，失败后无提醒；`collections.rejected` 连重试都没有（全仓无 `UPDATE collections`，仅登记）。

### 1.6 S5 镜像验证/分配

- 表：`image_verifications`（每次 verify 追加一行，`0011:51-59`）、`image_assignments`（房间维度分配，`0011:72-83`）、`image_channels`（`0011:62-69`）。
- 语义：verify 同步检查分片 sha256/size、总大小、格式 magic、manifest digest 与 Ed25519 签名；任一失败把 artifact 退回 `manifest_ready`（`crates/api/src/api/admin/images.rs:607-800`）。分配要求 artifact 已 `published`（`images.rs:992-1040`）。
- 权限：verify/publish/assign 均 `image:manage`（`images.rs:616` 等），读 `image:read`。
- **不可行动的原因**：验证失败只有 `image_verifications` 一行 + 审计 `image.verify`；没有“这个镜像验证失败需要修”的待办，也没有重试提示。

### 1.7 S6 节点配置下发失败（P0 只有日志，需 P1 补信号）

- 表：`agent_configs`（`0012:6-19`，四作用域见 `0017:19-47`）；下行队列 `device_downlink`（`0008:10-29`，未 ACK 行 `delivered_at IS NULL`，背压索引 `0008:26-29`）。
- 下发：`crates/api/src/api/admin/agent_configs.rs:296-321` —— 对每个受影响槽位 `enqueue_downlink_for_slot`，无存活设备时 `warn!` 并返回 false；响应只给 `pushed_slots` 计数（`agent_configs.rs:180-210`）。
- **不可行动的原因**：失败没有落库（只有日志 + `device_downlink` 排队状态），跨副本/重启后不可复盘。工单若覆盖此项，P0 只能先做“写配置时 `pushed_slots < expected` 生成工单”，或 P1 新增下发记录表。

### 1.8 S7 审计/安全

- `audit_logs`：`0003_audit.sql:2-16`（actor/action/resource_type/resource_id/detail/ip/created_at）；IP 列 `0005_audit_ip.sql:1-2`。写入口 `crates/db/src/audit.rs:18-43`，handler 统一走 `crates/api/src/api/admin/audit.rs:56-80` 的 `record_or_log`（失败只记日志不影响主流程）。
- 现有审计 action 命名是 `<资源>.<动作>`（如 `room.create`、`schedule_event.replay`、`image.verify`、`transfer.retry`、`room.set_event_rules`），工单审计应沿用。
- **不可行动的原因**：审计是 append-only 事后记录，没有“待处理/指派/关闭”；登录失败/锁定只存在 Redis 计数器（`crates/cache/src/cache.rs:148-191`），过期即丢，不能当安全工单数据源（P1 需 `security_events` 表）。

---

## 2. 后台 worker 约定（工单 worker 必须遵守）

### 2.1 装配方式（两款现成范式）

- 启动处统一在 `crates/api/src/lib.rs:51-70`：`if config.xxx_enabled { let _x = module::spawn(state.clone()); }`，JoinHandle detach（生命周期跟随进程）。工单 worker 应照抄这里的开关+日志风格。
- 配置集中在 `crates/api/src/config.rs:52-90`（`schedule_*`、`device_presence_*`、`machine_snapshot_keep` 等），启动时做范围校验（如 `config.rs:182-243`）。
- 业务专属 repo 挂在 `AppState`：`crates/api/src/state.rs:38-63`（每个 repo 持 `PgPool` 克隆，`state.rs:74-98` 装配）。工单应新增 `pub tickets: TicketRepo`。

### 2.2 两个 worker 的具体模式

| 维度 | presence worker（`crates/api/src/presence.rs`） | schedule worker（`crates/api/src/scheduler.rs`） |
|---|---|---|
| spawn | `spawn(state) -> JoinHandle`（`presence.rs:179-217`） | `spawn(state) -> JoinHandle`，另起启动重建/周期重建子任务（`scheduler.rs:87-158`） |
| 循环 | `tokio::time::interval` + `MissedTickBehavior::Delay`（`presence.rs:200-204`） | 同（`scheduler.rs:96-101`） |
| 每 tick | 先扫状态翻转并落库，再处理到期投递（`presence.rs:203-215`） | `tick_once`（扫描）+ `deliver_once`（投递）（`scheduler.rs:96-128`） |
| 幂等键 | DB 唯一约束 `(serial,current_state,source_updated_at)`（`0013:23`；`crates/db/src/presence.rs:84-105` `ON CONFLICT ... DO NOTHING`） | `schedule_events.event_key UNIQUE`（`0009:167`；`crates/db/src/schedule.rs:1951-2007` 事务内 `ON CONFLICT (event_key) DO NOTHING` + 同事务插投递行） |
| 失败重试 | 指数退避 `min(300s, base*2^(n-1))`（`presence.rs:162-171,383-425`） | `retry_or_fail` 同构（`scheduler.rs:500-537`） |
| 多副本 | **DB 约束 + `FOR UPDATE SKIP LOCKED` 领取**（`crates/db/src/presence.rs:123-147`，租约 60s） | **Redis `SET NX EX` 锁**互斥整轮（`scheduler.rs:46-58,161-170,336-347`；`crates/cache/src/cache.rs:221,238`），DB 唯一键兜底；投递领取未用 SKIP LOCKED（`crates/db/src/schedule.rs:2078-2106`），靠锁保证不重复 |
| SSE/广播 | 落库后 `broadcast::Sender` 本地推送（`presence.rs:254-266`；`state.rs:50-51,73`），SSE 以查库为准（`device_events.rs:1-10`） | 无 SSE；前端轮询/复用人查库 |
| 失败隔离 | 单条失败只 `warn!`，不打断循环（`presence.rs:203-215`） | 单轮错误只 `error!`，下一 tick 继续（`scheduler.rs:105-126`） |

### 2.3 对工单 worker 的直接结论

1. **幂等第一优先级是 DB 约束，不是锁**：锁只降负载（Redis 故障时 scheduler 也有短暂空隙），部分唯一索引才是跨副本的硬保证 —— 见 P0 DDL 的 `dedupe_key` 部分唯一索引。
2. tick 默认 60s 级即可（机器 fast 10s 会放大扫描成本，工单语义不需要秒级）。
3. 与 presence 一致：**首轮建立基线**（记录每 serial/room 的“已知状态快照”到 `tickets_worker_state` 或直接用源表最新状态+事件水位），避免重启后把历史问题全部重放一遍。
4. 单条信号处理失败不得影响整轮；工单创建失败只 `tracing::warn!`/`error!`。

---

## 3. RBAC：现状与“加 ticket 权限”必须改的清单

### 3.1 现状

- 权限点常量 + `ALL`：`crates/domain/src/rbac.rs:9-55`（21 个，`rbac.rs:95` 断言 `ALL.len() == 21`）。
- 内置角色：`admin`/`editor`/`viewer`（`rbac.rs:57-62`），`admin` 受保护（`crates/api/src/api/admin/roles.rs:95-119` 不可改系统角色权限；不可删除 `roles.rs:143-175`；DB 层 `crates/db/src/roles.rs:176` 仅删 `is_system=FALSE`）。
- 权限判定：精确匹配，无通配（`crates/auth/src/permission.rs:27-39`）；`require_permission` 见 `permission.rs:72-84`，API 胶水 `crates/api/src/auth.rs:24-52`。
- 权限缓存：key `rbac:v{version}:user:{id}:perms`（`permission.rs:42-70`，TTL 300s），变更时 `INCR rbac:version`（`crates/cache/src/cache.rs:92-96`，`permission.rs:100-103`）。
- 启动补齐：`bootstrap::ensure_bootstrap_admin` 调 `grant_all_permissions_to_admin()`，有新增则 bump 代际（`crates/api/src/bootstrap.rs:11-16`；SQL `crates/db/src/roles.rs:185-198`）。
- 迁移播种范式（每次新增权限都照抄）：
  - `0002_seed.sql:4-45`：权限 `INSERT ... ON CONFLICT (code) DO NOTHING`；角色 seed；`admin` CROSS JOIN 全量授权；`editor`/`viewer` 定向授权。
  - `0003_audit.sql:18-27`、`0009_schedule.sql:200-223`、`0010_device_room.sql:10-27`、`0011_storage.sql:113-124`：同样的“权限 + admin 全量 + viewer/editor 只读”三段式。
- 新增权限行后，`GET /admin/permissions` 直接读 DB 表（`crates/db/src/roles.rs:88`），**无需改代码**即可在角色编辑页可见。

### 3.2 加 ticket 权限的精确改动清单

1. `crates/domain/src/rbac.rs`：
   - 新增常量 `TICKET_READ = "ticket:read"`、`TICKET_CREATE = "ticket:create"`、`TICKET_HANDLE = "ticket:handle"`、`TICKET_MANAGE = "ticket:manage"`（放在 `IMAGE_MANAGE` 之后，`rbac.rs:30` 附近）。
   - 追加进 `ALL`（`rbac.rs:53` 之后），并把 `rbac.rs:95` 的断言从 `21` 改为 `25`。
   - 新增一个断言 ticket 四个常量都在 `ALL` 中的测试（与 `rbac.rs:80-96` 同风格）。
2. 新迁移 `migrations/0018_tickets.sql`（见 §6）：插权限行、admin 全量、viewer/editor 只读（`ticket:read`）、可选 `operator` 角色及其授权。
3. `bootstrap`：无需改代码；部署重启后若 admin 缺新权限会自动补齐并 bump 缓存代际（`bootstrap.rs:11-16`）。
4. 缓存注意：已有会话的权限缓存最长 300s（`permission.rs:7`）；viewer/editor 缓存的旧列表不含 `ticket:read`，上线后最多 5 分钟内仍可能 403。若要“立即生效”，发布说明要求重启全部副本（每个副本启动时 `grant_all...` 有新增即 bump）。
5. 前端类型/文档：`crates/api/src/api/openapi.rs` 增加路径与 schema，再刷新 `frontend/docs/openapi.json` 并执行 `python3 frontend/docs/openapi-types.py`（`frontend/docs/openapi-types.py:1-8`；脚本入口 `frontend/package.json:23`）——后端 PR 至少要保证 spec 可生成，前端团队再跑 `pnpm run type-check`。
6. 权限语义建议（P0 最小集）：
   - `ticket:read`：列表/详情/事件流只读。
   - `ticket:create`：手工建单（不自动建单）。
   - `ticket:handle`：认领/转派给自己、评论、推进状态（open→in_progress→resolved→closed）、取消。
   - `ticket:manage`：转派给任意人、改严重级、重开、删除/合并（P1）、维护规则。
   - 内置角色：`operator`（`is_system=FALSE`，便于管理员后续调整）：`ticket:read/create/handle` + `device:read` + `area:read` + `room:read`。

---

## 4. API 约定（新接口必须对齐）

- 路由注册：`crates/api/src/api/mod.rs:33-65`（外层中间件/安全头/CORS），`mod.rs:111-126`（`/api/v1` 挂载；`/admin` 统一 `Cache-Control: no-store`）。后台路由集中在 `crates/api/src/api/admin/mod.rs:25-150`；模块声明在 `admin/mod.rs:1-15`。
- 鉴权：handler 签名 `AuthUser { id }` extractor（`crates/api/src/api/extractors.rs:11-66`），第一行 `authorization::require_permission(&state, id, permissions::XXX).await?`（示例 `machines.rs:101`、`schedule_events.rs:122`）。
- 请求体：`ApiJson<T>` 把反序列化错误转成统一 `AppError`（`extractors.rs:101-120`）；PATCH 可空字段用 `deserialize_nullable`（`admin/mod.rs:152-165`）。
- 分页：`PageQuery { page, page_size, keyword, status }`（`crates/api/src/api/validation.rs:4-15`）→ `validation::page_params`（默认 page=1、page_size=20、上限 100，`crates/domain/src/validation.rs:22-23,220-235`）→ `Page<T>{items,total,page,page_size}`（`crates/domain/src/models.rs:152-169`）。
- 错误：`AppError` 九种（`crates/api/src/error.rs:14-100`），响应体 `{ "error": { "code", "message" } }`（`error.rs:146-181`）；5xx 只回通用文案。业务冲突用 `AppError::conflict`（409），权限用 403。
- 审计：写操作统一 `super::audit::record_or_log(&state, actor_id, action, resource_type, Some(id), json!({...}), ip.as_deref()).await`（`crates/api/src/api/admin/audit.rs:56-80`；示例 `rooms.rs:161`、`storage.rs:301`、`images.rs:785-800`）。
- OpenAPI：spec 是手写 JSON（`crates/api/src/api/openapi.rs:37-1402` 的 info/schemas，`openapi.rs:1402` 起是 paths），测试断言路径存在（`openapi.rs:2606` 起 `spec_lists_core_paths_and_security`），另断言 tags 含各模块（`openapi.rs:2680-2712`）。新增 ticket 需要：新 tag、components/schemas、paths、测试断言、`/docs` 无需改。
- 测试范式：集成测试放 `crates/api/tests/*.rs`，`#[ignore = "requires PostgreSQL and Redis; ..."]`，用 `tower::ServiceExt::oneshot` 打 router（`crates/api/tests/device_presence.rs:1-11`、`integration.rs:1-10`），配置 `run_migrations: true` 时由 `Db::migrate()` 跑迁移（`crates/db/src/db.rs:51-54`，迁移目录 `sqlx::migrate!("../../migrations")`）。

---

## 5. 迁移约定

- 下一个编号：**`0018_tickets.sql`**（当前最大 `0017_agent_config_device.sql`）。迁移内嵌在二进制，启动时自动跑（`crates/db/src/db.rs:51-54`；`README.md:1134-1140`）。
- ID：业务主键一律 `UUID PRIMARY KEY DEFAULT gen_random_uuid()`（`0001:13`、`0009:9`）；流水/投递/快照用 `BIGSERIAL`（`0013:12`、`0014:10`、`0008:11`）——工单主表建议 UUID（对外可引用、可合并不换号），`ticket_events` 用 `BIGSERIAL`（内部时序）。
- 时间：一律 `TIMESTAMPTZ NOT NULL DEFAULT now()`；本地时区取 `areas.timezone`（`0009:13`），事件类表存时区快照（`0009:170` 的 `timezone`）。
- 更新触发器：带 `updated_at` 的表复用 `set_updated_at()`（`0001:4-10`，用法 `0009:22-25`）。
- 约束风格：枚举值一律 `TEXT ... CHECK (...)`（`0014:26-29`、`0013:19`），不用 PG enum；跨表规则用部分唯一索引（`0009:43`、`0012:18`、`0017:36-47`）。
- 幂等 SQL：`INSERT ... ON CONFLICT (...) DO NOTHING`（`0013` 注释、`crates/db/src/presence.rs:84-105`）；重复执行迁移安全（`ON CONFLICT (code) DO NOTHING` 是权限播种的硬规范）。
- 权限播种：三段式（权限行 + admin CROSS JOIN + viewer/editor 定向），见 §3.1；`operator` 角色用 `INSERT INTO roles ... ON CONFLICT (code) DO NOTHING`，`is_system=FALSE` 以保留管理员可编辑性。

---

## 6. P0 具体方案

### 6.1 DDL（`migrations/0018_tickets.sql` 草案）

```sql
-- 工单：人工/自动事件的唯一处理入口。
-- 自动工单的幂等由 dedupe_key 部分唯一索引保证（仅 open/in_progress 唯一），
-- 历史工单不受影响（关闭后可对同一问题再开新单）。
CREATE SEQUENCE IF NOT EXISTS tickets_code_seq;

CREATE TABLE tickets (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code             TEXT NOT NULL UNIQUE,           -- TK-YYYYMMDD-000123（人类可读单号）
    kind             TEXT NOT NULL CHECK (kind IN (
        'machine_health','device_offline','schedule_delivery',
        'device_presence_delivery','storage_transfer','image_verify',
        'agent_config_downlink','security','manual')),
    severity         TEXT NOT NULL DEFAULT 'P2' CHECK (severity IN ('P0','P1','P2','P3')),
    status           TEXT NOT NULL DEFAULT 'open' CHECK (status IN (
        'open','in_progress','resolved','closed','cancelled')),
    title            TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    -- 信号来源：source_type 对应表名/事件名，source_id 为源行主键（字符串统一）
    source_type      TEXT,
    source_id        TEXT,
    dedupe_key       TEXT,                            -- 自动单幂等键；手工单为 NULL
    room_id          UUID REFERENCES rooms(id) ON DELETE SET NULL,
    area_id          UUID REFERENCES areas(id) ON DELETE SET NULL,
    device_slot      TEXT,
    serial           TEXT,
    assignee_id      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_by       UUID REFERENCES users(id) ON DELETE SET NULL, -- NULL = 系统自动生成
    acknowledged_at  TIMESTAMPTZ,
    resolved_at      TIMESTAMPTZ,
    closed_at        TIMESTAMPTZ,
    resolution       TEXT,
    occurrence_count INT NOT NULL DEFAULT 1,
    last_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    metadata         JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (status NOT IN ('resolved','closed') OR resolved_at IS NOT NULL)
);

-- 自动单去重：同一 dedupe_key 在“未结束”状态下最多一张；关闭后可再开新单。
CREATE UNIQUE INDEX tickets_dedupe_active_key
    ON tickets (dedupe_key)
    WHERE dedupe_key IS NOT NULL AND status IN ('open','in_progress');
CREATE INDEX tickets_status_updated_idx ON tickets (status, updated_at DESC);
CREATE INDEX tickets_assignee_idx      ON tickets (assignee_id, status);
CREATE INDEX tickets_room_idx          ON tickets (room_id, created_at DESC);
CREATE INDEX tickets_kind_severity_idx ON tickets (kind, severity, status);
CREATE INDEX tickets_source_idx        ON tickets (source_type, source_id);

DROP TRIGGER IF EXISTS tickets_set_updated_at ON tickets;
CREATE TRIGGER tickets_set_updated_at
    BEFORE UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 处理轨迹：创建/认领/转派/状态变更/评论/再次发生。
CREATE TABLE ticket_events (
    id          BIGSERIAL PRIMARY KEY,
    ticket_id   UUID NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN (
        'created','assigned','unassigned','status_changed','commented','occurred')),
    actor_id    UUID REFERENCES users(id) ON DELETE SET NULL,   -- NULL = 系统
    from_status TEXT,
    to_status   TEXT,
    note        TEXT NOT NULL DEFAULT '',
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    source_type TEXT,
    source_id   TEXT,                                           -- 自动单的这次源行（occurred 去重）
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ticket_events_ticket_idx ON ticket_events (ticket_id, id);
-- 同一源行只追加一次 occurred 轨迹（多副本/重复扫描幂等）
CREATE UNIQUE INDEX ticket_events_occurrence_key
    ON ticket_events (ticket_id, source_type, source_id)
    WHERE kind = 'occurred' AND source_id IS NOT NULL;
```

权限与角色（同一迁移尾部，照抄 §3.1 三段式）：

```sql
INSERT INTO permissions (code, name, description) VALUES
    ('ticket:read',   '查看工单', '查看工单列表、详情与处理轨迹'),
    ('ticket:create', '创建工单', '手工创建工单'),
    ('ticket:handle', '处理工单', '认领/转派给自己、评论、推进状态与取消'),
    ('ticket:manage', '管理工单', '转派给任意人、调整严重级、重开与删除')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

INSERT INTO roles (code, name, description, is_system) VALUES
    ('operator', '值班运维', '处理设备/机器/投递类工单', FALSE)
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code IN ('ticket:read','ticket:create','ticket:handle',
                                 'device:read','area:read','room:read')
WHERE r.code = 'operator'
ON CONFLICT DO NOTHING;

-- viewer/editor 只读
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code = 'ticket:read'
WHERE r.code IN ('viewer','editor')
ON CONFLICT DO NOTHING;
```

### 6.2 管理端 API 清单（`/api/v1/admin`）

| 方法/路径 | 权限 | 请求 | 响应 |
|---|---|---|---|
| `GET /tickets` | `ticket:read` | `status,kind,severity,assignee_id,room_id,area_id,keyword,from,to,page,page_size` | `Page<Ticket>` |
| `POST /tickets` | `ticket:create` | `{kind,severity,title,description,room_id?,area_id?,device_slot?,assignee_id?}` | `201 Ticket` + 审计 |
| `GET /tickets/{id}` | `ticket:read` | — | `{ "ticket": Ticket, "events": TicketEvent[] }` |
| `PATCH /tickets/{id}` | `ticket:handle`（改状态/自领/评论）/`ticket:manage`（转他人/改级） | `{status?,assignee_id?,severity?,title?,description?,resolution?,note?}`，可空字段用 `deserialize_nullable` | `Ticket` + 审计 |
| `POST /tickets/{id}/comments` | `ticket:handle` | `{note}` | `201 TicketEvent` |
| `POST /tickets/{id}/assign` | `ticket:manage` | `{assignee_id}` | `Ticket` + 审计 |

- 状态流转：`open → in_progress → resolved → closed`；`open/in_progress → cancelled`；`resolved → in_progress`（重开）由 `ticket:manage` 执行。`resolved` 必须带 `resolution`，`closed` 必须已有 `resolved_at`（DDL CHECK 与 handler 双重校验）。
- 自动单不自动关闭（避免“信号恢复即关单”掩盖根因）；P1 再加“源信号恢复 N 分钟自动 resolved（保留手工 close）”。
- SSE（P1）：`GET /tickets/stream?after_id=`，复用 `device_events.rs:1-10` 的查库轮询模式（多副本安全），P0 先用列表轮询。
- 列表 JSON 示例：`{"items":[{"id":"…","code":"TK-20261101-000007","kind":"machine_health","severity":"P1","status":"in_progress","title":"room-101 磁盘告警","room_id":"…","device_slot":"room-101","assignee_id":"…","occurrence_count":3,"last_seen_at":"…","created_at":"…"}],"total":1,"page":1,"page_size":20}`。

### 6.3 自动工单 worker 规则

新建 `crates/api/src/ticket_worker.rs`（`spawn` 范式照抄 §2.1），config 增加 `TICKETS_AUTO_ENABLED`(默认 true)、`TICKET_TICK_SECONDS`(默认 60，范围 10–600)、`TICKET_DEVICE_OFFLINE_MINUTES`(默认 10)、`TICKET_HEALTH_CONSECUTIVE`(默认 2)、`TICKET_LOOKBACK_HOURS`(默认 24)。

| 规则 | 源查询（每 tick） | dedupe_key | 默认严重级 | 抑制/冷却 |
|---|---|---|---|---|
| machine_health | 每 serial 最新两条快照（`DISTINCT ON (serial) ... ORDER BY collected_at DESC`）；最新 `health_status != 'ok'` 且连续 `TICKET_HEALTH_CONSECUTIVE` 条非 ok | `machine_health:{serial}` | warning→P2，critical→P1 | 连续计数过滤 10s 抖动；`health_score` 下降不作为触发（P1 再加） |
| device_offline | `device_session.state='offline'` 且 `updated_at <= now() - interval '{N} minutes'`，且所在房间/区域 `is_active` | `device_offline:{serial}` | P2 | N 默认 10 min；教室正常关机误报复盘后再加时段策略（§8） |
| schedule_delivery | 近 lookback 内 `schedule_event_deliveries.status='failed'` JOIN `schedule_events` | `schedule_delivery:{room_id}:{target}:{local_date}` | P2 | 按房间+目标+本地日期聚合（一次 webhook 故障不刷屏）；`skipped` 不入单 |
| device_presence_delivery | `device_presence_deliveries.status='failed'`（有 `updated_at`，`0013:27-42`） | `presence_delivery:{target_url}:{local_date}` | P3 | 同上聚合 |
| storage_transfer | `transfers.status IN ('failed','expired')`（`0011:86-98`） | `transfer:{transfer_id}` | P2 | 一任务一单；retry 成功后 P1 自动 resolved |
| image_verify | 最新 `image_verifications.ok=false` 的 artifact（`0011:51-59`） | `image_verify:{artifact_id}` | P2 | 每 artifact 一单，重复失败只累加 occurrence |
| agent_config_downlink | P0 不做（无持久信号）；P1 用 `device_downlink.delivered_at IS NULL AND created_at < now()-X` | `downlink:{serial}:{kind}` | P2 | §1.7 |
| security | P0 不做（需 `security_events` 表） | — | — | §1.8 |

写入策略（多副本安全，硬保证）：

```sql
INSERT INTO tickets (code, kind, severity, title, description, source_type, source_id,
                     dedupe_key, room_id, area_id, device_slot, serial, created_by, metadata)
VALUES (..., NULL)                     -- created_by NULL = 系统
ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL AND status IN ('open','in_progress') DO NOTHING
RETURNING id;
-- 返回 0 行：说明已有未结束工单 → 累加发生次数并追加 occurred 轨迹
UPDATE tickets
   SET occurrence_count = occurrence_count + 1, last_seen_at = now(),
       severity = CASE WHEN severity_rank($2) < severity_rank(severity) THEN $2 ELSE severity END -- severity_rank 为实现侧伪函数
 WHERE dedupe_key = $1 AND status IN ('open','in_progress');
INSERT INTO ticket_events (ticket_id, kind, source_type, source_id)
VALUES (..., 'occurred', $2, $3)
ON CONFLICT (ticket_id, source_type, source_id)
    WHERE kind='occurred' AND source_id IS NOT NULL DO NOTHING;
```

- `code` 用 `nextval('tickets_code_seq')` + 日期生成，`UNIQUE` 兜底；跨副本不会重号。
- worker tick 用 Redis `try_lock("ticket:worker:tick", max(tick*4,60))` 降负载（`crates/cache/src/cache.rs:221,238`），但**不依赖它保证正确性**。
- 无法映射到具体房间/设备时仍建单（`room_id` 为空），metadata 记录原始错误文本，避免信号丢失。

### 6.4 审计与轨迹

- 人工操作写 `audit_logs`：`ticket.create` / `ticket.assign` / `ticket.status` / `ticket.comment`（`resource_type="ticket"`，`resource_id=ticket.id`，detail 含 `from_status/to_status/assignee_id`），走 `record_or_log`（`crates/api/src/api/admin/audit.rs:56-80`）。
- 系统自动单**不写审计**（无 actor），只写 `ticket_events.kind='created'/'occurred'`，`actor_id=NULL`，避免污染审计语义（审计 = 人的管理动作，见 §1.8）。
- 工单状态变更同时写 `ticket_events`（处理轨迹的事实来源），审计只作合规旁路。

### 6.5 代码改动面（供 Lead 估工，本次均未改）

| 文件 | 动作 |
|---|---|
| `migrations/0018_tickets.sql` | 新增（§6.1） |
| `crates/domain/src/rbac.rs` | 改：4 个常量、`ALL`、断言 21→25、新测试 |
| `crates/domain/src/ticket.rs`（或 `models.rs`） | 新增：`Ticket`/`TicketEvent`/状态与严重级枚举（`FromStr`/`Display` 风格照 `crates/domain/src/storage.rs:227-266`） |
| `crates/db/src/tickets.rs` + `crates/db/src/lib.rs:6-20` | 新增 repo（`insert_if_absent`、`bump_occurrence`、`list`、`get`、`update_status`、`add_event`），`lib.rs` 加 `pub mod tickets;` |
| `crates/api/src/state.rs:30-99` | 加 `pub tickets: TicketRepo` 并装配 |
| `crates/api/src/api/admin/tickets.rs` + `admin/mod.rs:1-15,25-150` | 新增 handler 与路由 |
| `crates/api/src/ticket_worker.rs` + `crates/api/src/lib.rs:51-70` | 新增 worker 与开关 |
| `crates/api/src/config.rs:52-90,180-250` | 新增配置字段与范围校验 |
| `crates/api/src/api/openapi.rs` | 新 tag/schemas/paths + 测试断言（`openapi.rs:1402`,`2606`） |
| `crates/api/tests/tickets.rs` | 新增集成测试（§6.6） |
| `frontend/docs/openapi.json`、`frontend/src/api/schema.ts` | 重新生成（`frontend/docs/openapi-types.py:1-8`） |
| `README.md`、`docs/design/ticket-design.md` | 文档（前端/契约由 Lead 统一） |

### 6.6 测试清单

1. 单测：`rbac.rs` 断言更新为 25 且 ticket 常量在 `ALL` 中（改 `rbac.rs:95`）。
2. 单测：worker 纯函数（dedupe_key 生成、严重级映射、连续计数/离线阈值、本地日期聚合），风格照 `crates/api/src/presence.rs:477-628`、`crates/api/src/scheduler.rs:1010-1471`（注入 `now`，不依赖 DB/Redis）。
3. 集成（`crates/api/tests/tickets.rs`，`#[ignore]` + `tower::ServiceExt::oneshot`，照 `tests/device_presence.rs:1-11`）：
   - 无 `ticket:read` → 403；`ticket:handle` 可推进状态、`ticket:create` 可建单、`ticket:manage` 可转派。
   - 手工建单写 `audit_logs`（`action="ticket.create"`）；响应含 `code`。
   - 去重：同一 `dedupe_key` 连续两次自动信号 → 1 张工单、`occurrence_count=2`、`ticket_events` 只 1 条 `occurred`（部分唯一索引生效）。
   - 关闭后可再开：`status='closed'` 后再插入同 `dedupe_key` 成功（部分唯一索引不覆盖终态）。
   - 状态机校验：`resolved` 缺 `resolution` → 400/409；`closed` 前未 resolved → 409。
   - 分页/过滤（status/kind/severity/assignee/room）与 `page_size` 上限 100。
4. OpenAPI 测试：`crates/api/src/api/openapi.rs` 的 `spec_lists_core_paths_and_security` 增加 `/admin/tickets` 等断言；tags 断言加 `Tickets`。
5. 迁移冒烟：集成测试 `Db::migrate()` 已覆盖（`crates/db/src/db.rs:51-54`）；`scripts/test-integration.sh` 会跑 ignored 用例。
6. 前端：后端合入后刷新 spec → `python3 frontend/docs/openapi-types.py` → `pnpm run type-check`（本环境不执行）。

---

## 7. 风险与取舍（决策点）

1. **告警噪音（最高风险）**：fast 档 10s 上报（`docs/machine-telemetry.md:133-139`）会让 `ok↔warning` 抖动。必须连续 N 次 + 单 `dedupe_key` 聚合 + 发生次数累加；不要每快照建单。
2. **教室正常关机 ≠ 故障**：`device_presence_events` 只表达翻转，不表达“应当在线”。P0 建议：`device_offline` 规则默认以 10 分钟阈值 + 房间/区域 `is_active` 过滤，**并在设计文档中明确“默认关闭该规则”作为可配置项**；P1 引入 `devices.expected_online` 或按课表时段判定（`schedule_occurrences` 可查当前是否有课，`crates/db/src/schedule.rs:1774-...`）。
3. **多副本重复建单**：只靠 Redis 锁不够（Redis 故障降级），必须 `dedupe_key` 部分唯一索引 + `ON CONFLICT DO NOTHING` + `ticket_events` 唯一索引双保险（§2.3、§6.3）。
4. **时区/静默时段**：区域时区在 `areas.timezone`（`0009:13`），课表事件已存时区快照（`0009:170`）；聚合“按天”必须用房间所属区域时区的本地日期（参考 `crates/domain/src/schedule.rs` 的 `local_date_of`/`local_to_utc`）。静默时段 P0 用环境变量/关闭规则，P1 落 `ticket_rules` 表（按区域/房间配置）。
5. **权限缓存 5 分钟窗口**：新权限对已登录用户不会立即生效（`crates/auth/src/permission.rs:7,54`）。发布顺序建议：先迁移（幂等）→ 再滚动重启（bootstrap 补 admin 权限并 bump，`bootstrap.rs:11-16`）→ 再发前端。
6. **自动关单会掩盖根因**：P0 不做自动 resolved；`occurrence_count/last_seen_at` 让处理人看到“又发生了”，而不是被静默关闭。
7. **数据量**：`ticket_events` 是增长最快的表；P1 加保留期/归档（参考 `machine_snapshots` 的保留策略 `crates/db/src/machine.rs:336-350`）。工单主表本身很小。
8. **不要复用 `audit_logs` 当工单轨迹**：审计字段没有状态机、不可更新（append-only 语义），混用会让“谁改了什么”和“问题处理到哪”两类查询互相污染。
9. **S6/S7 缺口**：配置下发失败与安全事件目前无持久信号（§1.7/§1.8）；若首期就要求覆盖，需先补 `agent_config_delivery`/`security_events` 表，成本明显高于 S1–S5。
10. **手工单与自动单的产品边界**：建议同一 `tickets` 表 + `kind='manual'`，避免两套列表/权限；自动单允许被人工转派与关闭。

---

## 8. 给 Lead 的开放问题

1. 首期工单是否只做“事件聚合 + 人工处理”（本文 P0），还是同时要“通知/升级/SLA”（P1）？
2. `device_offline` 自动建单默认开还是关？如果开，误报（教室关机）由谁维护抑制规则？
3. 工单是否必须绑定房间/机器（外键强约束）？跨房间（如区域级网络故障）如何表达？
4. 通知渠道：复用现有邮件（`crates/api/src/email.rs:14-70`）还是 webhook/企微？P0 是否先不做？
5. 是否需要工单 SSE 实时流（可复用 `device_events` 的查库轮询范式），还是前端轮询列表即可？
6. 关闭后的工单与再次发生：新建单（当前 DDL 语义）还是重开旧单（会丢失“这次发生”的时间线）？
7. 前端页面归属：新菜单 `/tickets`，还是并入现有“机器总览/设备事件”页的抽屉？后端接口不受影响，但 `ticket:read` 的菜单显隐需要前端权限点先行。
