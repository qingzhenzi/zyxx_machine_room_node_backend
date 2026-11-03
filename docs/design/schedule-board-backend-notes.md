# 跨房间排课：后端能力盘点与两条演进路径

> 基线：`main` @ `12bae07`（2026-11-01 12:15）。只读调研，未改任何业务代码。
> 契约基线：`docs/schedule-design.md`（v2 IANA 时区章节 §10 覆盖前文）+ `migrations/0009_schedule.sql`。
> 目标场景：用户原话「需要可以编排所有的房间，可视化操作」；候选界面 A 周网格 / B 资源时间轴 / C 全房间矩阵。
> 本文只回答「这些交互落到后端要什么」，不涉及 UI 设计（见前端同事产物）。

---

## 1. 现状盘点

### 1.1 数据模型（`migrations/0009_schedule.sql`）

| 表 | 关键列 | 索引 / 约束 | 语义 | 引用 |
| --- | --- | --- | --- | --- |
| `areas` | `timezone`（IANA）、`parent_id`、`webhook_url`（0013 追加） | `areas_parent_idx` | 区域=时区来源 | `0009:8-25`、`0013:10` |
| `rooms` | `area_id NOT NULL`、`device_slot`、`capacity` | `rooms_area_idx`、槽位部分唯一索引 | 调度最小单位，绑 1 个主设备槽 | `0009:29-49` |
| `schedules` | `room_id NOT NULL`、`start_date/end_date`、`is_active` | `schedules_room_idx (room_id,start_date,end_date)`；**无重叠互斥约束、无 metadata 列** | 一份课表=一个房间的一段时间 | `0009:52-70` |
| `schedule_weekly_slots` | `schedule_id`、`day_of_week 1..7`、`TIME`、`course_name/teacher TEXT`、`room_id`(调课) | `(schedule_id,day_of_week,start_time)` | 周循环模板 | `0009:72-86` |
| `schedule_exceptions` | `(schedule_id,exception_date)` **UNIQUE**、`kind cancel/replace` | 一天一条 | 整天停课/整天替换 | `0009:89-102` |
| `schedule_exception_slots` | `exception_id`、`TIME`、`room_id` | `(exception_id,start_time)` | `replace` 当天全量时段 | `0009:105-117` |
| `schedule_occurrences` | `schedule_id`、`room_id`(实际上课房间)、`start_at/end_at TIMESTAMPTZ`、`source weekly/custom`、`source_slot_id` | `(room_id,start_at)`、`(schedule_id,start_at)`、`(start_at,end_at)`、`UNIQUE(schedule_id,source,source_slot_id,occurrence_date)` | 物化实例（事件引擎唯一数据源） | `0009:120-138` |
| `schedule_event_rules` | `room_id`、6 种 `trigger_kind`、`config JSONB` | `(room_id) WHERE enabled` | 按房间的触发规则 | `0009:141-161` |
| `schedule_events` | `event_key` **UNIQUE**、`room_id`、`timezone` 快照、`schedule_id/occurrence_id/rule_id` | 房间时间、创建时间索引；三个外键 **ON DELETE SET NULL** | 事件落库（幂等键） | `0009:165-182` |
| `schedule_event_deliveries` | `(event_id,target)` UNIQUE、`status`、退避重试 | 待投递部分索引 | 每一目标一行 | `0009:184-199` |

设备侧补充：`room_devices`（子机，`0015`）与 `rooms.device_slot`（主机）互斥，由触发器保证；课表/事件只认主机。
全库**没有** `terms/courses/teachers/classes` 任何表（`migrations/` 全量 grep 确认），课程与教师是纯文本列。

当前数据模型是严格 room-scoped：`schedules.room_id` 是唯一归属，`room_override` 只是实例层面的「这节课去别处上」。

### 1.2 领域类型与纯函数（`crates/domain/src/schedule.rs`）

| 类型/函数 | 用途 | 引用 |
| --- | --- | --- |
| `Area / Room / Schedule / WeeklySlot / ScheduleException(Slot) / Occurrence` | DTO；`Schedule` 带只读 `timezone`、`room_name`，**无 area 字段** | `:29,:70,:114,:148,:224,:296` |
| `ScheduleExceptionKind{Cancel,Replace}`、`OccurrenceSource{Weekly,Custom}`、`TriggerKind`（6 种） | 枚举 | `:190,:274,:314` |
| `Validation`：`MAX_WEEKLY_SLOTS=200`、`MAX_EXCEPTION_SLOTS=100`、`MAX_OCCURRENCE_RANGE_DAYS=366`、`page_size≤100` | 硬上限 | `validation.rs:22,275-278` |
| `validate_weekly_slots` | 仅校验**同一课表同一天**时段不重叠（跨房间/跨课表/教师冲突均不查） | `validation.rs:436-473` |
| `validate_exception_slots` | 单天时段不重叠 | `validation.rs:477-517` |
| `expand_occurrences(schedule, weekly, exceptions, from, to)` | **纯函数**：clamp 到生效区间 → cancel 跳过 / replace 只用覆盖时段 / 否则按 ISO 周展开 | `schedule.rs:578-668` |
| `local_to_utc / local_date_of / local_clock_of / local_rfc3339` | IANA 时区转换；DST 不存在时刻→400，重复时刻取较早 | `:673-712` |

注意文档漂移：模块头注释仍写「时区统一用 UTC 偏移分钟，不引入 chrono-tz」（`schedule.rs:6`），实际已用 `chrono_tz`（`:9`），以代码为准。

### 1.3 仓储方法（`crates/db/src/schedule.rs`，约 2063 行）

| 方法 | 行为 | 引用 |
| --- | --- | --- |
| `ScheduleRepo::list(room_id?, page, page_size)` | 唯一过滤条件是 `room_id`；**不支持 area/日期/active 过滤** | `:1032-1071` |
| `get / create / update / delete` | `create` 不物化实例；`delete` 级联实例 | `:1073,:1088,:1113,:1140` |
| `weekly_slots(schedule_id)` | 只按课表读；调课房间名靠 `LEFT JOIN rooms` | `:1149,:571-600` |
| `replace_weekly_slots` | **事务内 DELETE 全量 + 逐条 INSERT**→本次保存后 slot UUID 全变 | `:1155-1197` |
| `exceptions / upsert_exception / delete_exception` | upsert=整天一条 + 全量替换 slots | `:1199,:1210-1274,:1276` |
| `rebuild_occurrences(schedule_id, horizon)` | 事务：`FOR UPDATE OF s` → 展开 → `DELETE ... start_at > now()` → 批量 INSERT（`ON CONFLICT DO NOTHING`） | `:1297-1409`（锁 `:1310`，删 `:1348`，插入 `:1367+`） |
| `rebuild_all_occurrences` | 串行遍历 `WHERE is_active` 的课表 | `:1412-1424` |
| `rebuild_area_occurrences(area_id)` | 区域时区变更后按区域串行重建 | `:1426-1447` |
| `occurrences(schedule_id,…)` / `room_occurrences(room_id,…)` | 对外查询仅这两种维度 | `:1449,:1465` |
| `occurrences_starting / _ending / _overlapping(from,to)` | **全房间**时间窗查询；`overlapping` 供事件引擎，`starting/ending` 无调用方；均无 HTTP 暴露 | `:1482,:1498,:1514` |
| `EventRuleRepo::enabled_rules()` | 全量启用规则 + 房间/区域快照；**不校验 rooms.is_active** | `:1623-1684` |
| `ScheduleEventRepo::insert_if_absent` / `list` / `replay_event` / `retry_delivery` | 事件幂等插入（同事务写投递行）；列表按 room/kind/时间过滤，**无 area 过滤** | `:1691,:1744,:1912,:1927` |

SQL 片段常量：`SCHEDULE_COLUMNS`(`:538`)、`OCCURRENCE_COLUMNS`(`:542`)、`OCCURRENCE_JOINS`(`:547`，时区取自「课表→源房间→区域」)。

### 1.4 HTTP 接口（`crates/api/src/api/admin/{areas,rooms,schedules,schedule_events}.rs`，路由 `admin/mod.rs:42-107`）

| 方法 | 路径 | 权限 | 备注 | 引用 |
| --- | --- | --- | --- | --- |
| GET | `/areas`、`/rooms` | `area:read`/`room:read` | 分页、`keyword`、`include_inactive`；rooms 支持 `area_id` | `areas.rs:103`、`rooms.rs:92` |
| POST/PATCH/DELETE | `/areas/{id}`、`/rooms/{id}` | `*:manage` | 区域改时区会**同步重建**该区域全部课表；**房间改 area_id 不重建** | `areas.rs:183-287`、`rooms.rs:181-269` |
| GET/POST | `/schedules` | `schedule:read/manage` | 列表仅 `room_id` 过滤，page_size≤100 | `schedules.rs:156,178` |
| PATCH/DELETE | `/schedules/{id}` | `schedule:manage` | 改日期后事务外重建 | `schedules.rs:221,283` |
| GET/PUT | `/schedules/{id}/weekly-slots` | read/manage | GET 单课表；PUT 全量替换并重建 | `schedules.rs:309,320` |
| GET | `/schedules/{id}/exceptions` | read | | `:353` |
| PUT/DELETE | `/schedules/{id}/exceptions/{date}` | manage | 整天一条，upsert 后重建 | `:370,408` |
| GET | `/schedules/{id}/occurrences?from&to` | read | 默认今天起 7 天、≤366 天 | `:440` |
| GET | `/rooms/{id}/occurrences?from&to` | read | 房间维度（含调课进来的课） | `:468`、路由 `mod.rs:74` |
| POST | `/schedules/{id}/rebuild` | manage | 手动重建（**无 Redis 锁**） | `:499` |
| GET/PUT | `/rooms/{id}/event-rules` | read/manage | 全量替换规则 | `schedule_events.rs:74,86` |
| GET | `/schedule-events`、`/{id}` | `schedule:event:read` | 按 room/kind/时间过滤，无 area | `:117,163` |
| POST | `/schedule-events/{id}/replay`、`.../deliveries/{id}/retry` | `schedule:event:manage` | 重放/重试 | `:184,212` |

实测：`GET /api/v1/admin/occurrences?from&to` → **404**（无跨房间聚合端点，§4）。

### 1.5 事件引擎与投递（`crates/api/src/scheduler.rs`）

- 启动：`spawn` → 立即一次全量重建 + 每 `SCHEDULE_REBUILD_INTERVAL_SECONDS`（默认 3600s）全量重建（`scheduler.rs:133-152`、`config.rs:190-193`）；tick 默认 15s（`config.rs:182-185`）。
- 每次 tick：抢 Redis 锁 → `enabled_rules()` → `occurrences_overlapping(now-窗口-12h, now+最大提前量)`（**全房间**）→ 按 `occurrence.room_id` 分桶 → 命中的规则生成事件（`scheduler.rs:172-268`，查询 `:193-197`）。
- 事件键：`occ:{rule_id}:{occurrence_id}:{kind}`（`:716-718`）；每日定时 `daily:{rule_id}:{local_date}`（`:721-723`）。
- 投递目标：`pubsub` / `downlink`（房间主设备槽）/ `webhook`（房间 → 区域 → 全局回落）；每目标一行，指数退避，不可达置 `skipped`（`:377-551`）。
- 幂等靠 `event_key UNIQUE` + 重叠扫描窗口。

### 1.6 权限与审计

- 权限点全局，无资源范围：`schedule:read`、`schedule:manage`、`schedule:event:read/manage`、`area/room:*`、`device:*`（`crates/domain/src/rbac.rs:9-50`；`0009:201-211`）。
- 内置角色 `admin/editor/viewer`；viewer/editor 只有读权限（`0009:218-224`）。没有「按区域/按教师」授权模型。
- 审计：每次写请求一行 `audit_logs`，`resource_type='schedule'`、`resource_id=schedule_id`，`detail` 只记摘要（如 `{"count":3}`），无字段级 diff、无批量归组（`schedules.rs:196-212` 等；`audit.rs:20-30` 只支持 action/resource_type 过滤）。
- `room.update` 审计记 `area_id/timezone`（`rooms.rs:250-264`），但**不会**因换区域触发重建。

### 1.7 六个关键问题的结论（带证据）

**Q1 能否一次拿到「某时间范围内全部房间的排课」？**
不能直接用于可视化。仓储层存在全房间时间窗查询 `occurrences_starting/ending/overlapping`（`db:1482/1498/1514`，走 `schedule_occurrences_time_idx`），其中 `occurrences_overlapping` 被事件引擎调用，`starting/ending` 无调用方；HTTP 只有 per-schedule 与 per-room 两种（`schedules.rs:440,468`）。且 `OccurrenceView` 不含 `room_name/room_code/area_id/area_name/schedule_name`（`schedules.rs:92-110`），前端必须再拉 `/rooms` 自己拼。`GET /admin/occurrences` 实测 404。

**Q2 能否拿到周模板全量（跨房间）？**
不能。`weekly_slots` 只接受 `schedule_id`（`db:1149`）；`GET /schedules` 只返回 schedule 元数据、不含 slots，且每页 ≤100、无 area/日期过滤（`db:1032`、`validation.rs:22`）。拿到 N 个房间的模板需 1 次列表 + N 次 `weekly-slots`。

**Q3 调课（`room_override`）后实例与事件归属？**
实例 `room_id = room_override.unwrap_or(schedule.room_id)`（`db:1367`），`schedule_id` 仍是**源课表**。因此：`GET /rooms/B/occurrences` 能看到调进 B 的课（`db:1465` `WHERE o.room_id`）；B 的课表模板/详情页看不到（模板仍挂 A 的 schedule）；事件按 `occurrence.room_id` 分桶（`scheduler.rs:201-206,238-241`）→ 触发 **B 的**规则（语义正确）。但实例时区取自「源课表→源房间→区域」（`db:547`）→ 跨区域调课时展示时区错位（实际 `start_at` 也是按源时区算的）。

**Q4 单日覆盖能否作用于其他房间？**
只能通过「覆盖时段自带的 `room_id`」把当天某节改到别的房间（`0009:112`）。不能取消/替换其他房间的课（覆盖属于某份课表）。`replace` 粒度是**整份课表的整天**：一天只能一条覆盖（`0009:97` UNIQUE），且会**替换当天全部周模板时段**（`schedule.rs:599-620`），不能只改当天一节课。`cancel` 同样是整份课表整天停课。

**Q5 并发/重建语义？**
写请求提交后**事务外**逐课表重建（`schedules.rs:142-156`）；重建只重写 `start_at > now()` 的实例（`db:1297,1348`），过去的实例与事件引用保持原样。跨课表**无批量事务**，N 个房间=N 次请求=N 次重建。同课表并发写由 `FOR UPDATE OF s` 串行（`db:1310`）。HTTP `POST /schedules/{id}/rebuild` 不抢 Redis 锁（`schedules.rs:499`），与周期全量重建（有锁，`scheduler.rs:553`）可并发，最坏情况重复 DELETE/INSERT 与死锁重试。**`POST /schedules` 不物化实例**（`schedules.rs:178-232`），新房间的课最迟等下一个周期重建（≤3600s）才出现。

**Q6 其它硬事实**
- 模板保存是**全量替换**（`db:1155`）：每次保存 slot UUID 全变 → 前端 key 抖动、`source_slot_id` 引用失效、事件键（基于 occurrence id）无法稳定。
- `schedules` 允许同房间多份重叠课表（无互斥约束）→ 重复实例、重复事件在数据层无兜底。
- `is_active` 不挡事件：`rebuild_occurrences` 与 `enabled_rules` 都不看 `is_active`/`rooms.is_active`；`rebuild_all` 只是跳过 inactive 课表，其**已存在的**实例仍会被扫描（`db:1412-1416`、`:1623-1684`、`scheduler.rs:172-222`）。停用房间/课表后会继续触发。
- 实例与事件的关联靠 `ON DELETE SET NULL`（`0009:171-173`）+ 每次重建删实例 → 现网 4 条历史事件的 `occurrence_id/schedule_id` 已全部为 NULL（§4），只剩 payload 里的旧 UUID。
- 事件扫描窗口 `max(60s, tick*4)`（`scheduler.rs:47,586`），只处理 `(now-窗口, now]`；进程停机超过窗口的触发永久丢失（无补偿扫描）。
- `occurrences_starting/ending` 在仓库外无调用（`grep` 仅定义处），属备用/死代码。

---

## 2. 能力缺口清单

影响：A=周网格（列=周几/行=节次），B=资源时间轴（行=房间/X=时间），C=全房间矩阵（行=房间/列=天×节次）。
严重度：P0 阻塞主交互 / P1 需绕行但体验明显受损 / P2 可后置。

| # | 缺口 | 事实与证据 | 影响 | 严重度 |
| --- | --- | --- | --- | --- |
| G1 | 无「时间窗 × 全部房间」读模型 | 全房间查询只在仓储内部（`db:1514`）；HTTP 404 | A B C | P0 |
| G2 | 无跨房间周模板全量读 | 模板只按 schedule 读（`db:1149`）；列表不含 slots（`db:1032`） | A C | P0 |
| G3 | 列表接口无法按区域/日期/active 过滤，页上限 100 | `ScheduleQuery` 仅 `room_id/page/page_size`（`schedules.rs:28-34`）；`validation.rs:22` | A B C | P1 |
| G4 | 无批量写接口/事务 | 所有写都是单课表（`schedules.rs:178-499`）；跨房间必须 N 请求 | A B C | P0 |
| G5 | 批量写触发 N 次重建，非原子 | 每次写后同步 `rebuild_occurrences`（`schedules.rs:145`）；跨房间无一致性边界 | A B C | P0 |
| G6 | 无冲突检测（房间/教师/班级/跨课表） | 校验只有同课表同日时段重叠（`validation.rs:460-473`）；无任何 overlap 查询 | A B C | P0（用户明确要「查冲突」） |
| G7 | 课程/教师/班级实体缺失 | `course_name/teacher` 是 TEXT（`0009:78-79`）；无班级概念 | A B C | P1 |
| G8 | 学期/版本/发布概念缺失 | `schedules` 只有 `start_date/end_date`，无 term/revision/publish（`0009:52-64`） | A B C | P1 |
| G9 | 跨房间调课语义是「半边」 | 实例迁到目标房间，但模板仍属源课表（`db:1367`）；源房间视角看不到调出，目标房间模板页看不到调入 | B C | P1 |
| G10 | 单日覆盖面过粗 | 一天一条、整天替换/整天停课，无法单节移动/改时长（`0009:89-117`、`schedule.rs:599-620`） | B C | P0（B 的拖拽改时长） |
| G11 | 无稳定条目 ID | 模板全量替换（`db:1155`），slot UUID 每次保存重生成 | A B C | P1 |
| G12 | 事件键依赖 occurrence.id，重建会换 id | `scheduler.rs:716`；`db:1348` DELETE+INSERT；现网事件 4/4 链接已 NULL | A B C（影响提醒） | P0 |
| G13 | 停用不生效 + 停机丢触发 | `db:1412`、`db:1623`、`scheduler.rs:172,586` 未见 active 过滤/补偿 | A B C | P1 |
| G14 | 无撤销/历史 | 无 revision/软删；编辑模板不回写过去实例（`db:1348` 只删未来） | A B C | P1 |
| G15 | 权限无区域/教师粒度 | `rbac.rs:9-50` 全局点；`0009:201` 无范围 | A B C | P2 |
| G16 | 审计粒度为请求摘要 | `detail={"count":n}`（`schedules.rs:337-345`）；无 diff、无批量归组 | A B C | P2 |
| G17 | 重建性能：全量串行 | `rebuild_all_occurrences` 逐课表（`db:1412`）；每个写后同步重建 | A B C | P1（房间多时） |
| G18 | 时区语义：模板时间按源房间区域解释 | `OCCURRENCE_JOINS` 取源房间区域（`db:547`）；跨区调课错位 | B C | P1 |
| G19 | 房间换区域不重建 | `rooms.rs:181-269` 无 rebuild 调用（对比 `areas.rs:266`） | A B C | P2（数据正确性） |
| G20 | 无「房间默认课表」概念 | 房间可以没有 schedule → 棋盘上没有可编排的槽；无唯一约束防止重复建 | A B C | P1 |
| G21 | 无多房间课程/合班表达 | 一条 slot 一个 `room_id`（`0009:80`） | C | P2 |
| G22 | 无节次（period）字典 | 只有 `TIME`；A 的「行=节次」需前端自行按时间分桶或新建节次表 | A C | P2 |

---

## 3. 两条演进路径

### 3.1 路径 1：薄聚合，不改核心模型

目标：不动 `0009` 既有表语义，只新增「跨房间读模型 + 批量写 + 冲突预检」三类接口，让前端能组合出 A/B/C 的「能看、能改」版本。

#### 3.1.1 新增接口

> 下表路径中的 `/admin` 段即 `/api/v1/admin`（与现有 admin 路由一致）；§3.2.4 的表格省略该段，含义相同。

| # | 方法/路径 | 权限 | 入参 | 出参要点 |
| --- | --- | --- | --- | --- |
| 1 | `GET /admin/schedule-board` | `schedule:read` | `from,to,area_id?,include_inactive?` | 一次返回：`areas[] → rooms[]`（含 device_slot/is_active）、`occurrences[]`（含 `schedule_id, source, source_slot_id, room_id, room_name, area_id, area_name, start_at, end_at, start_local, course_name, teacher, display_timezone`）、`templates[]`（schedule + weekly_slots 摘要，供「改模板」跳转） |
| 2 | `GET /admin/schedules/templates` | `schedule:read` | `area_id?,from?,to?` | 全量 `schedule + weekly_slots + exceptions`（一屏一次拉全，替代 1+N 请求） |
| 3 | `POST /admin/schedules/ensure` | `schedule:manage` | `{room_ids[],start_date,end_date,name}` | 幂等：缺课表的房间补一份默认课表；返回 `{created[],existing[]}`；需 DB 侧防重（见 3.1.4） |
| 4 | `PUT /admin/schedules/weekly-slots/batch` | `schedule:manage` | `{items:[{schedule_id,slots[]}]}` | 单事务写全部；提交后逐课表重建；返回 `{rebuilt:[{schedule_id,occurrences}],failed:[]}` |
| 5 | `PUT /admin/schedules/exceptions/batch` | `schedule:manage` | `{items:[{schedule_id,date,kind,slots[]}]}` | 跨房间单日停课/替换 |
| 6 | `GET /admin/schedule-conflicts` | `schedule:read` | `from,to,area_id?` | 内存展开（复用 `domain::expand_occurrences`，不落库）→ `conflicts[]`：`room_overlap / teacher_overlap / duplicate_schedule / unassigned_room` |
| 7 | `GET /schedules` 增强过滤 | `schedule:read` | 增加 `area_id,is_active,from,to` | 支撑棋盘初始化与「哪些房间没课表」 |
| 8 | `POST /admin/schedule-conflicts/preview`（可选） | `schedule:manage` | 临时条目集合 | 拖拽未保存时预检（不写库） |

`GET /admin/schedule-board` 的核心 SQL（新增仓储方法，走现有 `schedule_occurrences_time_idx`）：

```sql
-- 有效上课房间 r_eff = o.room_id（可能是调课目标）；源课表房间 r_src 决定模板时区
SELECT o.id, o.schedule_id, o.room_id,
       r_eff.code AS room_code, r_eff.name AS room_name,
       r_eff.area_id, a_eff.name AS area_name,
       s.name AS schedule_name, s.room_id AS source_room_id,
       a_src.timezone AS display_timezone,
       o.occurrence_date, o.start_at, o.end_at,
       o.course_name, o.teacher, o.source, o.source_slot_id, o.note
FROM schedule_occurrences o
JOIN schedules s ON s.id = o.schedule_id
JOIN rooms  r_src ON r_src.id = s.room_id
JOIN areas  a_src ON a_src.id = r_src.area_id
JOIN rooms  r_eff ON r_eff.id = o.room_id
JOIN areas  a_eff ON a_eff.id = r_eff.area_id
WHERE o.start_at < $to AND o.end_at > $from
  AND ($area_id::uuid IS NULL OR r_eff.area_id = $area_id OR r_src.area_id = $area_id)
ORDER BY r_eff.area_id, r_eff.code, o.start_at, o.id;
```

冲突检测（不依赖物化窗口，可查任意未来日期）：批量加载该区域全部 `schedules + weekly_slots + exceptions`（3 条 `WHERE s.room_id = ANY(...)` 查询），对每份课表调 `expand_occurrences`（`domain/schedule.rs:578`），把结果按 `room_id` 与 `teacher` 分组做区间重叠扫描（O(n log n)）。不写库、不受 `SCHEDULE_HORIZON_DAYS` 限制；500 房间 × 14 天 × 6 节 ≈ 4.2 万条，单请求内存与耗时可控。

#### 3.1.2 批量写的事务与重建

- 批量写本身放在**一个 `PgPool` 事务**里（多条 `replace_weekly_slots`/`upsert_exception` 逻辑复用）；事务内不做展开。
- 提交后逐课表重建（复用 `rebuild_occurrences`），失败项收集进 `failed[]` 返回部分成功；接受「写成功、重建失败」的既有语义（`schedules.rs:142-156` 已如此）。
- 批量接口用请求级 Redis 锁串行化（`schedule:board:write`，复用 `cache.try_lock`，`scheduler.rs:567` 已有实现），避免两个管理员同时整体铺课；单课表写仍由 `FOR UPDATE` 保证。
- 重建代价：每课表 O(horizon × weekly_slots)，批量 N 房间即 N 倍；返回值里带 `rebuilt_count` 便于前端提示。

#### 3.1.3 三个候选界面在路径 1 下的能力边界

| 交互 | A 周网格 | B 资源时间轴 | C 全房间矩阵 | 说明 |
| --- | --- | --- | --- | --- |
| 查看全部房间排课 | ✅ | ✅ | ✅ | `GET /schedule-board` |
| 查看周模板（跨房间） | ✅ | 一般 | ✅ | 接口 2 |
| 拖拽块换房间 | ⚠️ | ⚠️ | ⚠️ | 写源课表 slot 的 `room_override`；目标房间模板里看不到这条课 |
| 拖拽改时长/单节移动 | ❌ | ❌ | ❌ | 只能整天 `replace`，且要手工重建当天其余时段 |
| 单日停课 | ✅ | ✅ | ✅ | batch exceptions |
| 框选批量铺课（模板级） | ⚠️ | ❌ | ✅ | 接口 4，N 房间一次请求，但非原子（重建逐课表） |
| 跨周复制/按天批量例外 | ❌ | ❌ | ⚠️ | 例外是「每天一条整天替换」，O(房间×天) 且会切断模板关系 |
| 查冲突 | ✅ | ✅ | ✅ | 接口 6/8，内存展开 |
| 撤销 | ❌ | ❌ | ❌ | 无版本；只能前端反向重放写请求（又触发重建） |
| 改历史（昨天/已过日期） | ❌ | ❌ | ❌ | 重建只写未来（`db:1348`） |
| 稳定条目 ID / 事件幂等 | ❌ | ❌ | ❌ | 模板全量替换 + occurrence id 重建（G11/G12） |
| 按区域/教师授权 | ❌ | ❌ | ❌ | 权限模型不变 |

#### 3.1.4 路径 1 的配套加固（仍属「薄」改动）

1. `0018_schedule_board.sql`（若先落地路径 1）：`CREATE UNIQUE INDEX schedules_one_active_per_room ON schedules (room_id) WHERE is_active;`（支撑 ensure 幂等；上线前需先清历史重复数据）。
2. 事件键改为稳定来源：`occ:{rule_id}:{schedule_id}:{source_slot_id}:{occurrence_date}:{kind}`——可消除「重建后重复提醒」，但 slot id 在全量替换下仍会变，只能缓解不能根治（根治要条目实体，见路径 2）。
3. 事件引擎补 `JOIN schedules s ON ... WHERE s.is_active` 与 `JOIN rooms r WHERE r.is_active`（G13），并在启动时回扫 `max(窗口, 停机时长)` 内的触发（G13 的丢失问题）。
4. `rooms` 换 `area_id` 时重建该房间的未来实例（G19）。

#### 3.1.5 路径 1 评价

- 优点：1 周多可交付；前端三候选界面都能先跑起来并验证产品形态；完全复用既有写入语义，回归风险低；对现网数据零迁移。
- 缺点：语义债留在数据模型里——「批量非原子、调课半边、无撤销、无稳定 ID、无教师/班级冲突、事件幂等靠补丁」；房间数上百后批量重建与 `/schedules` 分页拼装会持续放大。
- 无法达成：C 的「一次编排一学期 + 任意撤销」、B 的「拖拽改时长/跨房间移动的原子提交」、跨房间冲突硬拦截（教师维度）都无法用接口层优雅解决。

### 3.2 路径 2：重构为全局排课实体（学期 + 课表版本 + 条目）

目标：把「房间是课表 owner」改成「课表是全局编排，房间是条目的一个可改属性」，让 A/B/C 的所有交互都落在同一个可事务、可版本化的实体上。

#### 3.2.1 目标模型

```
terms（学期）──< schedule_plans（学期课表，可按 area 划分全域/区域）
                     ├──< plan_entries（排课条目：day_of_week + TIME + room_id? + course/teacher/class）
                     ├──< plan_adjustments（单日调整：cancel / move / replace，按条目）
                     └──< schedule_plan_revisions（发布快照，JSONB，用于回滚/撤销）

courses / teachers / classes（主数据，条目引用 + 名称快照）
rooms / areas（不变）
schedule_occurrences（唯一物化表，事件引擎继续用；新增 plan_entry_id / timezone / teacher_id / class_id）
```

关键决定：
- **一条 entry = 一个房间**。合班/分组后期加 `plan_entry_rooms`，本期不做（见 §5 Q7）。
- **发布才物化**：草稿编辑不碰 `schedule_occurrences`；`publish` 事务内做「快照 revision + 同步物化」。编辑已发布课表 = 编辑草稿态条目，下次发布生效；回滚 = 用 revision 恢复条目并重新发布。
- **稳定 ID**：条目 id 在多次发布之间不变 → occurrence 用 `(plan_entry_id, occurrence_date)` **upsert（保留 id）**，事件键 `plan:{entry_id}:{date}:{kind}` → 重复提醒与断链从根上消失。
- **时区**：条目本地 `TIME` 按**条目所在房间的区域时区**解释（跨区调课语义正确）；时间点仍存 UTC；`schedule_occurrences.timezone` 存快照，展示不再依赖 JOIN。

#### 3.2.2 DDL 草案（`migrations/0018_schedule_plan.sql`：新增表 + 对实例表做向后兼容的列扩展，不改 0009 已有列语义）

> 若路径 1 的加固迁移先合入，本迁移顺延为 `0019_schedule_plan.sql`。必须新增迁移而不是改 0009：`crates/db/src/db.rs:51-52` 用 `sqlx::migrate!`，会校验已应用迁移的 checksum，改 0009 会让现网启动直接报错。

```sql
-- 全局排课（schedule v3）：主数据 + 学期 + 课表计划 + 条目 + 单日调整 + 发布快照。
-- 与 0009 的 room-scoped 课表并存：本迁移只新增表，并给 schedule_occurrences 增加兼容列。

CREATE TABLE terms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_date <= end_date)
);
CREATE INDEX terms_range_idx ON terms (start_date, end_date) WHERE is_active;

CREATE TABLE courses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '',           -- 前端课块颜色
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE teachers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,  -- 可选：与登录账号关联
    email TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX teachers_user_idx ON teachers (user_id) WHERE user_id IS NOT NULL;

CREATE TABLE classes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    grade TEXT NOT NULL DEFAULT '',
    headcount INT NOT NULL DEFAULT 0 CHECK (headcount >= 0),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 课表计划：一个学期一份（area_id NULL = 全域）；草稿可直接编辑，发布才物化实例。
CREATE TABLE schedule_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    term_id UUID NOT NULL REFERENCES terms(id) ON DELETE RESTRICT,
    area_id UUID REFERENCES areas(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'published', 'archived')),
    published_revision INT,
    published_at TIMESTAMPTZ,
    published_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (status <> 'draft' OR published_at IS NULL)   -- 草稿不得带发布时间
);
CREATE UNIQUE INDEX schedule_plans_global_scope_key ON schedule_plans (term_id) WHERE area_id IS NULL;
CREATE UNIQUE INDEX schedule_plans_area_scope_key ON schedule_plans (term_id, area_id) WHERE area_id IS NOT NULL;

-- 排课条目：课程/教师/班级用「外键 + 名称快照」，主数据改名或删除不破坏已发布课表。
CREATE TABLE plan_entries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id UUID NOT NULL REFERENCES schedule_plans(id) ON DELETE CASCADE,
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,   -- NULL = 待定教室
    day_of_week SMALLINT NOT NULL CHECK (day_of_week BETWEEN 1 AND 7),
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_id UUID REFERENCES courses(id) ON DELETE SET NULL,
    course_name TEXT NOT NULL,
    teacher_id UUID REFERENCES teachers(id) ON DELETE SET NULL,
    teacher_name TEXT NOT NULL DEFAULT '',
    class_id UUID REFERENCES classes(id) ON DELETE SET NULL,
    class_name TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_time < end_time)
);
CREATE INDEX plan_entries_plan_idx ON plan_entries (plan_id, day_of_week, start_time);
CREATE INDEX plan_entries_room_idx ON plan_entries (room_id, day_of_week, start_time) WHERE room_id IS NOT NULL;
CREATE INDEX plan_entries_teacher_idx ON plan_entries (teacher_id, day_of_week, start_time) WHERE teacher_id IS NOT NULL;
CREATE INDEX plan_entries_class_idx ON plan_entries (class_id, day_of_week, start_time) WHERE class_id IS NOT NULL;

-- 单日调整：按「条目」维度，而不是按「整份课表的整天」。
CREATE TABLE plan_adjustments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id UUID NOT NULL REFERENCES schedule_plans(id) ON DELETE CASCADE,
    entry_id UUID REFERENCES plan_entries(id) ON DELETE CASCADE,  -- NULL = 当天整体停课
    adjust_date DATE NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('cancel', 'move', 'replace')),
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,          -- move：新房间
    start_time TIME,
    end_time TIME,
    course_name TEXT,
    teacher_id UUID REFERENCES teachers(id) ON DELETE SET NULL,
    teacher_name TEXT,
    note TEXT NOT NULL DEFAULT '',
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (kind <> 'replace' OR (start_time IS NOT NULL AND end_time IS NOT NULL AND start_time < end_time)),
    CHECK (kind <> 'move' OR room_id IS NOT NULL)
);
CREATE INDEX plan_adjustments_plan_date_idx ON plan_adjustments (plan_id, adjust_date);
CREATE UNIQUE INDEX plan_adjustments_entry_date_key ON plan_adjustments (entry_id, adjust_date) WHERE entry_id IS NOT NULL;
CREATE UNIQUE INDEX plan_adjustments_day_off_key ON plan_adjustments (plan_id, adjust_date) WHERE entry_id IS NULL;

-- 发布快照：撤销/回滚/审阅都基于它；不建条目版本表，避免跨版本 id 漂移。
CREATE TABLE schedule_plan_revisions (
    plan_id UUID NOT NULL REFERENCES schedule_plans(id) ON DELETE CASCADE,
    revision_no INT NOT NULL,
    entries JSONB NOT NULL,       -- 发布瞬间 plan_entries + plan_adjustments 全量快照
    note TEXT NOT NULL DEFAULT '',
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (plan_id, revision_no)
);

-- 与 0009 的物化表兼容：新增来源列；计划实例的 schedule_id 允许为 NULL。
ALTER TABLE schedule_occurrences
    ADD COLUMN plan_entry_id UUID REFERENCES plan_entries(id) ON DELETE SET NULL,
    ADD COLUMN timezone TEXT,
    ADD COLUMN teacher_id UUID REFERENCES teachers(id) ON DELETE SET NULL,
    ADD COLUMN class_id UUID REFERENCES classes(id) ON DELETE SET NULL,
    ALTER COLUMN schedule_id DROP NOT NULL,
    ALTER COLUMN source_slot_id DROP NOT NULL;
ALTER TABLE schedule_occurrences DROP CONSTRAINT schedule_occurrences_source_check;
ALTER TABLE schedule_occurrences ADD CONSTRAINT schedule_occurrences_source_check
    CHECK (source IN ('weekly', 'custom', 'plan'));
UPDATE schedule_occurrences o SET timezone = a.timezone
  FROM schedules s JOIN rooms r ON r.id = s.room_id JOIN areas a ON a.id = r.area_id
 WHERE s.id = o.schedule_id AND o.timezone IS NULL;
ALTER TABLE schedule_occurrences ALTER COLUMN timezone SET NOT NULL;
CREATE UNIQUE INDEX schedule_occurrences_plan_key
    ON schedule_occurrences (plan_entry_id, occurrence_date) WHERE plan_entry_id IS NOT NULL;
CREATE INDEX schedule_occurrences_teacher_idx
    ON schedule_occurrences (teacher_id, start_at) WHERE teacher_id IS NOT NULL;
-- 旧唯一键 (schedule_id, source, source_slot_id, occurrence_date) 保留：schedule_id 为 NULL 时不生效，计划实例靠上面的部分唯一索引。
-- 冻结 legacy 物化：灰度区域内该房间的 legacy 课表不再重建，历史实例保留。
ALTER TABLE schedules ADD COLUMN materialize BOOLEAN NOT NULL DEFAULT TRUE;
CREATE INDEX schedules_materialize_idx ON schedules (room_id) WHERE materialize AND is_active;

-- 已发布计划内禁止同房间/同教师时段重叠；草稿允许冲突（前端提示）。
-- 用触发器而不是 EXCLUDE 约束：需要按 plan.status 动态生效，且报错信息要能回给前端。
CREATE OR REPLACE FUNCTION plan_entries_reject_overlap()
RETURNS trigger AS $$
DECLARE
    conflict TEXT;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM schedule_plans p WHERE p.id = NEW.plan_id AND p.status = 'published') THEN
        RETURN NEW;   -- 草稿：允许冲突
    END IF;
    IF NEW.room_id IS NOT NULL THEN
        SELECT '房间 ' || r.name INTO conflict
        FROM plan_entries e JOIN rooms r ON r.id = NEW.room_id
        WHERE e.plan_id = NEW.plan_id AND e.id <> NEW.id
          AND e.room_id = NEW.room_id AND e.day_of_week = NEW.day_of_week
          AND tsrange(date '2000-01-03' + (e.day_of_week - 1) + e.start_time,
                      date '2000-01-03' + (e.day_of_week - 1) + e.end_time)
              && tsrange(date '2000-01-03' + (NEW.day_of_week - 1) + NEW.start_time,
                         date '2000-01-03' + (NEW.day_of_week - 1) + NEW.end_time)
        LIMIT 1;
        IF conflict IS NOT NULL THEN
            RAISE EXCEPTION '同一房间时段重叠：%', conflict;
        END IF;
    END IF;
    IF NEW.teacher_id IS NOT NULL THEN
        SELECT '教师 ' || t.name INTO conflict
        FROM plan_entries e JOIN teachers t ON t.id = NEW.teacher_id
        WHERE e.plan_id = NEW.plan_id AND e.id <> NEW.id
          AND e.teacher_id = NEW.teacher_id AND e.day_of_week = NEW.day_of_week
          AND tsrange(date '2000-01-03' + (e.day_of_week - 1) + e.start_time,
                      date '2000-01-03' + (e.day_of_week - 1) + e.end_time)
              && tsrange(date '2000-01-03' + (NEW.day_of_week - 1) + NEW.start_time,
                         date '2000-01-03' + (NEW.day_of_week - 1) + NEW.end_time)
        LIMIT 1;
        IF conflict IS NOT NULL THEN
            RAISE EXCEPTION '同一教师时段重叠：%', conflict;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER plan_entries_reject_overlap
    BEFORE INSERT OR UPDATE OF plan_id, room_id, teacher_id, day_of_week, start_time, end_time
    ON plan_entries FOR EACH ROW EXECUTE FUNCTION plan_entries_reject_overlap();

-- 其余新表统一 updated_at 触发器
CREATE TRIGGER terms_set_updated_at BEFORE UPDATE ON terms
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER courses_set_updated_at BEFORE UPDATE ON courses
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER teachers_set_updated_at BEFORE UPDATE ON teachers
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER classes_set_updated_at BEFORE UPDATE ON classes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER schedule_plans_set_updated_at BEFORE UPDATE ON schedule_plans
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER plan_entries_set_updated_at BEFORE UPDATE ON plan_entries
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER plan_adjustments_set_updated_at BEFORE UPDATE ON plan_adjustments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 新权限（沿用 0009 的写法：权限点 + admin 全量 + viewer/editor 只读）
INSERT INTO permissions (code, name, description) VALUES
    ('term:read',        '查看学期',       '查看学期列表与详情'),
    ('term:manage',      '管理学期',       '创建、修改、删除学期'),
    ('catalog:read',     '查看课程/教师/班级', '查看主数据'),
    ('catalog:manage',   '管理课程/教师/班级', '创建、修改、删除主数据'),
    ('plan:read',        '查看排课计划',   '查看学期课表、条目、单日调整与冲突'),
    ('plan:manage',      '编辑排课计划',   '创建计划、增删改条目与单日调整'),
    ('plan:publish',     '发布/回滚课表',  '发布课表版本、回滚历史版本')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin' AND p.code IN
      ('term:read','term:manage','catalog:read','catalog:manage','plan:read','plan:manage','plan:publish')
ON CONFLICT DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code IN ('term:read', 'catalog:read', 'plan:read')
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;

-- 灰度开关：按区域决定是否由 plan 驱动（见 §3.2.5）
CREATE TABLE schedule_plan_rollout (
    area_id UUID PRIMARY KEY REFERENCES areas(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

补充说明：
- `plan_entries` 的 overlap 触发器只覆盖「同 plan / 同周几 / 模板级」；**跨天与单日调整后的冲突**必须在 publish 前按展开结果再查一遍（单日调整可能把周三的课移到周四，模板级触发器看不到）；另外 `publish` 只是把 `schedule_plans.status` 改成 `published`，不会触发条目触发器，应用层必须先跑 `detect_conflicts` 再翻状态。
- 未来若把已发布计划的条目冻结（发布后不可直接改条目，只能新建草稿），可把触发器换成 `EXCLUDE USING gist` 约束（需要把 `day_of_week + TIME` 包成 IMMUTABLE 表达式列），性能更好；本期先用触发器。
- `schedule_plans` 的两个部分唯一索引只保证「全域一份」与「逐区域各一份」各自唯一；两者不能并存（同一学期既有全域计划又有区域计划会双重驱动），由应用层创建计划时校验。
- 回填 `schedule_occurrences.timezone` 能覆盖全表：`0009:122` 的 `schedule_id` 是 `ON DELETE CASCADE`，删除课表不会留下孤儿实例，所以不存在无法回填时区的行。
- 迁移是纯新增表 + `ALTER TABLE schedule_occurrences` 加可空列，PG 11+ 的 `ADD COLUMN` 无默认值不重写表；`SET NOT NULL` 建议拆成「回填 → `CHECK ... NOT VALID` → `VALIDATE CONSTRAINT` → 再收紧」两步执行，避免长锁。

#### 3.2.3 领域类型草案（`crates/domain/src/schedule_plan.rs`）

```rust
pub struct Term { id: Uuid, code, name, start_date: NaiveDate, end_date: NaiveDate,
                  is_active: bool, metadata: Value, created_at, updated_at }
pub struct Course { id, code, name, color: String, is_active, metadata, created_at, updated_at }
pub struct Teacher { id, code, name, user_id: Option<Uuid>, email: Option<String>,
                     is_active, metadata, created_at, updated_at }
pub struct ClassGroup { id, code, name, grade: String, headcount: i32,
                        is_active, metadata, created_at, updated_at }   // 避开 Rust 关键字

pub enum PlanStatus { Draft, Published, Archived }
pub struct SchedulePlan { id, term_id, area_id: Option<Uuid>, name, status: PlanStatus,
                          published_revision: Option<i32>, published_at: Option<DateTime<Utc>>,
                          published_by: Option<Uuid>, created_by: Option<Uuid>, created_at, updated_at }

pub struct PlanEntry {
    id, plan_id,
    room_id: Option<Uuid>, room_name: Option<String>, timezone: String, // 房间所属区域时区
    day_of_week: i16, start_time: String /*"HH:MM"*/, end_time: String,
    course_id: Option<Uuid>, course_name: String,
    teacher_id: Option<Uuid>, teacher_name: String,
    class_id: Option<Uuid>, class_name: String,
    note, metadata, created_at, updated_at,
}

pub enum AdjustmentKind { Cancel, Move, Replace }
pub struct PlanAdjustment { id, plan_id, entry_id: Option<Uuid>, adjust_date: NaiveDate,
                            kind: AdjustmentKind, room_id: Option<Uuid>, start_time: Option<String>,
                            end_time: Option<String>, course_name: Option<String>,
                            teacher_id: Option<Uuid>, teacher_name: Option<String>, note }

pub enum ConflictKind { RoomOverlap, TeacherOverlap, ClassOverlap,
                        UnassignedRoom, InactiveRoom, OutsideTerm }
pub struct Conflict { kind: ConflictKind, date: NaiveDate, start_at, end_at,
                      room_id: Option<Uuid>, room_name: Option<String>,
                      entry_ids: Vec<Uuid>, detail: String }

/// 纯展开：按条目所在房间的时区把本地 TIME 转 UTC；单日调整优先于模板。
pub fn expand_plan(plan: &SchedulePlan, entries: &[PlanEntry], adjustments: &[PlanAdjustment],
                   from: NaiveDate, to: NaiveDate) -> DomainResult<Vec<PlannedEntryOccurrence>>;
/// 冲突检测：同一批展开结果内做区间扫描，返回稳定排序的冲突列表。
pub fn detect_conflicts(occurrences: &[PlannedEntryOccurrence]) -> Vec<Conflict>;
```

与现有代码的关系：`expand_occurrences`（`schedule.rs:578`）保留给 legacy 课表；新函数与它共享时区/展开规则，但输入是「条目 + 调整」而不是「周模板 + 整天覆盖」。

#### 3.2.4 API 草案（全部挂 `/api/v1/admin`）

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET/POST | `/terms`、`/terms/{id}`（PATCH/DELETE） | `term:read/manage` | 学期 CRUD |
| GET/POST | `/courses`、`/courses/{id}` | `catalog:read/manage` | 课程主数据 |
| GET/POST | `/teachers`、`/teachers/{id}` | 同上 | 教师主数据 |
| GET/POST | `/classes`、`/classes/{id}` | 同上 | 班级主数据 |
| GET/POST | `/plans?term_id=&area_id=` | `plan:read/manage` | 计划列表/创建（自动带一份空条目集） |
| GET/PATCH | `/plans/{id}` | `plan:read/manage` | 计划详情（含 `published_revision`） |
| GET | `/plans/{id}/entries` | `plan:read` | 条目 + 调整（草稿态） |
| POST | `/plans/{id}/entries` | `plan:manage` | **批量**新增：`{entries:[...]}` → `{created:[...], conflicts:[...]}`（草稿允许冲突，返回警告） |
| PATCH/DELETE | `/plans/{id}/entries/{entry_id}` | `plan:manage` | 单条改/删（跨房间移动 = 改 `room_id`） |
| PUT | `/plans/{id}/entries` | `plan:manage` | 全量替换（事务 + 冲突预检；供「整周铺课」「复制周」使用） |
| POST | `/plans/{id}/entries/copy` | `plan:manage` | 复制周/复制计划：`{source_plan_id?, source_week?, target_week?}` |
| GET/PUT/DELETE | `/plans/{id}/adjustments/{date}` | `plan:read/manage` | 按日期批量写单日调整（条目级 cancel/move/replace） |
| GET | `/plans/{id}/conflicts?from=&to=&area_id=` | `plan:read` | 模板级 + 展开级冲突（含草稿），不落库 |
| GET | `/schedule-board?term_id&from&to&area_id` | `plan:read` | 棋盘聚合：区域→房间→实例（或草稿展开），与路径 1 同构，前端可无感替换 |
| POST | `/plans/{id}/publish` | `plan:publish` | 事务：写 revision 快照 → 同步物化实例（upsert，保留 id）→ 事件键稳定 |
| POST | `/plans/{id}/rollback` | `plan:publish` | `{revision_no}` → 恢复条目快照并重新发布 |
| GET | `/plans/{id}/revisions` | `plan:read` | 版本列表（撤销用） |
| GET | `/rooms/{id}/occurrences`、`/schedules/*` | 兼容保留 | 读时从 plan 投影，保证旧页面可用 |

#### 3.2.5 与 occurrences / 事件引擎的兼容与迁移策略

核心原则：**不动事件投递链路**（`schedule_events` / `deliveries` / `scheduler.rs` 的扫描与投递逻辑保持），只把「实例来源」从「room-scoped 展开」换成「plan 物化」，并让实例主键在多次发布/编辑之间稳定。

物化语义（`PublishPlan` 事务内）：

1. 校验草稿：`detect_conflicts` 硬冲突（同房间/同教师）→ 409，除非请求带 `force=true`（记录审计）。
2. 计算物化窗口：`[max(now, term.start_date), min(term.end_date, now + SCHEDULE_HORIZON_DAYS)]`（沿用现有 horizon 配置）。
3. 生成 `plan` 来源的实例记录：对每个 `(plan_entry_id, occurrence_date)` **UPSERT** 到 `schedule_occurrences`（`ON CONFLICT (plan_entry_id, occurrence_date) DO UPDATE SET start_at/end_at/room_id/course_name/teacher...`）→ **id 不变**；本次不再出现的未来实例 DELETE。
4. 过去的实例（`start_at <= now()`）一律不动：与现有 `db:1348` 语义一致，保护事件引用与历史。
5. 事件键改 `plan:{plan_entry_id}:{occurrence_date}:{trigger_kind}`（`scheduler.rs:716` 分支判断 `occurrence.plan_entry_id.is_some()`；legacy 分支保持旧键不变，零回归）。
6. 单日调整改变房间/时间时，因为 UPSERT 是同一行，事件键不变 → 已发的提醒不会重复，新的时间在下次查询生效；必要时对「改时间已跨过提前量」的情况显式发一条 `occurrence_rescheduled` 事件（可选，进 §5 Q6）。

数据迁移 / 回填（一次性 Rust 脚本 `scripts/migrate-schedule-to-plan.rs` 或 `cargo xtask`，而不是纯 SQL）：

| 步骤 | 内容 | 幂等键 |
| --- | --- | --- |
| 1 | 建 `terms`：按现有 `schedules` 的日期范围聚类（或人工指定目标学期） | `terms.code` |
| 2 | 每个学期建一份 `schedule_plans`（`area_id=NULL`，`status='published'`） | `(term_id, area_id)` 部分唯一索引 |
| 3 | 复制 `weekly_slots → plan_entries`：`room_id=COALESCE(w.room_id, s.room_id)`，`course/teacher` 落名称快照，`teacher_id/course_id=NULL` | 临时映射表 `legacy_slot_map(schedule_id, slot_id, entry_id)`，脚本可重跑 |
| 4 | 复制 `exceptions`：`cancel` → `plan_adjustments(entry_id=NULL, kind='cancel')`；`replace` → 用 `domain::expand_occurrences` 先算出当天被替换的条目集合，逐条写 `kind='replace'`（保留未出现的条目不动，比旧的「整天全量替换」更精确） | `(plan_id, adjust_date)` + 条目映射 |
| 5 | `publish`：物化到 `schedule_occurrences`（`source='plan'`），窗口与 legacy 一致 | `(plan_entry_id, occurrence_date)` |
| 6 | 冻结 legacy：`schedules` 增加 `materialize BOOLEAN NOT NULL DEFAULT TRUE`，迁移过的房间置 FALSE；`rebuild_all_occurrences`（`db:1412-1424`）增加 `WHERE is_active AND materialize`；删除这些房间**未来**的 legacy 实例（历史保留） | `schedules.id` |
| 7 | 校验：对同一窗口分别用 legacy `expand_occurrences` 与 `expand_plan` 展开，逐 `(room_id, occurrence_date, start_at)` 比对，全等才切读 | — |

灰度与回滚：

| 阶段 | 动作 | 出口标准 |
| --- | --- | --- |
| P0 | 只上路径 1 的只读聚合 + 冲突预览（0 迁移） | 三候选界面拿到真实数据 |
| P1 | 上 `0018`（纯新增表）+ 主数据 CRUD + 回填脚本 dry-run | 回填结果与 legacy 比对 0 差异 |
| P2 | 选 **1 个区域** 开 `schedule_plan_rollout.enabled=true`：该区域读写走 plan，legacy 只读 | 观察一个教学周：事件数、投递量、冲突告警一致 |
| P3 | 按区域扩散；`schedule-board` 对外服务所有区域 | 无 P0/P1 缺陷，审计可追溯 |
| P4 | legacy 写接口进入维护态（`materialize=false` 的课表禁止 PATCH weekly-slots），保留读接口一个版本周期 | 一个学期后删除 legacy 表 |

回滚：`schedule_plan_rollout.enabled=false` 即回到 legacy（legacy 表与实例未被破坏）；迁移前对 `schedules/schedule_weekly_slots/schedule_exceptions/schedule_occurrences` 做 `pg_dump` 兜底。

#### 3.2.6 0009 已上线数据的迁移风险

| 风险 | 具体说明 | 缓解 |
| --- | --- | --- |
| 迁移 checksum | 不能改 0009；只能新增 0018/0019（`db.rs:51-52`） | 已在方案中体现 |
| `schedule_occurrences` 改列锁表 | 大表 `SET NOT NULL` 会全表校验并持锁 | 回填后 `CHECK NOT VALID → VALIDATE`；低峰执行；列本身可空不影响读写 |
| 旧唯一键失效 | `schedule_id` 可空后旧唯一键对 plan 行不生效 | 新部分唯一索引 `(plan_entry_id, occurrence_date)`；所有物化路径统一 UPSERT |
| 事件引用断链 | 现网 4/4 历史事件 `occurrence_id/schedule_id` 已 NULL；若继续 DELETE+INSERT 会持续发生 | 物化改 UPSERT 保留 id；历史事件不回填（只保留 payload 快照） |
| 时区语义差异 | legacy 按「源课表房间区域」解释；新模型按「条目所在房间区域」解释 → 跨区调课数据会差 1 小时级 | 回填后逐条比对；把差异清单交业务确认，必要时修正条目 `room_id` |
| 跨区调课的模板归属 | legacy 中调课只在实例层，没有目标房间模板；迁移到 plan 条目后变成「目标房间的一条课」——**语义变好但数据形态变了** | 回填时以「实例展开结果」为准反推条目，而非直接复制模板（避免把调课当模板复制到源房间） |
| `replace` 覆盖整天 | legacy 的 replace 会替换当天全部时段；新模型按条目调整 | 回填必须用展开算法匹配「当天哪些条目被替换」，不能简单把 slots 全映射成 cancel |
| 重复/重叠课表 | 现网可能已有同房间重叠的多份 schedule（无互斥约束） | 回填前出报表；重叠数据需人工合并，脚本不自动合并 |
| 双写期两套事实源 | 旧页面（`SchedulesView.vue`/`ScheduleDetailView.vue`/`RoomSchedulesPanel.vue`）仍读写 legacy | 不做双写：legacy 在灰度区域只读，旧页面读时用 plan 投影；写入口收敛到新页面 |
| 回滚成本 | 一旦删除 legacy 表无法回滚 | 只冻结不删除；保留 `materialize` 开关一个学期 |

### 3.3 两条路径对比

| 维度 | 路径 1 薄聚合 | 路径 2 全局实体 |
| --- | --- | --- |
| 改动面 | 新增 6-8 个接口 + 3-4 个仓储方法 + 1 个索引迁移；0 表结构重写 | 0018 新表（9 张，含灰度开关）+ 改 `schedule_occurrences` 兼容列 + 新领域模块 + 新仓储 + 事件键分支 + 回填脚本 + 旧页面投影 |
| 风险 | 低（不改既有语义）；但事件幂等/调课语义是补丁，长期债 | 中高（物化语义、事件键、时区、回填一致性）；有 `materialize` 开关可回退 |
| 前端复杂度 | 中：需自己拼装 board、把「批量」拆成多次写、拖拽要处理 slot id 抖动 | 低-中：单一条目模型，拖拽=改 `room_id`/时间，批量=一次 PUT，撤销=revision |
| 支持 A 周网格 | 看 ✅ / 改模板 ✅ / 拖拽换房 ⚠️ / 撤销 ❌ | ✅（含撤销、单节调整） |
| 支持 B 时间轴 | 看 ✅ / 改时长 ❌ / 冲突高亮 ✅ | ✅（拖拽改时长=改条目时间或单日 replace） |
| 支持 C 全房间矩阵 | 看 ✅ / 模板级批量 ✅ / 按天批量 ⚠️ / 跨周复制 ❌ / 撤销 ❌ | ✅ |
| 学期/版本/发布 | 无（只有日期区间） | 有（term + 发布版本 + 回滚） |
| 课程/教师/班级主数据 | 无 | 有（含教师冲突硬拦截） |
| 事件重复提醒 | 只能缓解（换事件键） | 根治（稳定 entry id + UPSERT） |
| 工时粗估（人日） | domain 0.5 / db 2 / api 1.5 / 前端 3 / 测试 1 ≈ **8** | domain 4 / db 7.5 / api 5 / 前端 6.5 / 测试 3.5 ≈ **26.5** |
| 交付节奏 | 1 个迭代内可上 | 3-4 个迭代（含灰度一个教学周） |
| 推荐顺序 | **先做（只读部分）** | **随后作为写入模型的目标态** |

工时拆分（路径 2 内部）：

| 阶段 | domain | db | api | 前端 | 测试 | 小计 |
| --- | --- | --- | --- | --- | --- | --- |
| P1 主数据 + 0018 + 回填 | 1.5 | 2.5 | 1.5 | 2 | 1 | 8.5 |
| P2 计划/条目/物化 + 事件键 | 1.5 | 3 | 2 | 2.5 | 1.5 | 10.5 |
| P3 发布/回滚/单日调整/审计 | 1 | 2 | 1.5 | 2 | 1 | 7.5 |
| 合计 | 4 | 7.5 | 5 | 6.5 | 3.5 | **26.5** |

### 3.4 推荐

**推荐：路径 1 的只读部分先上，写入模型按路径 2 分阶段落地。** 不建议把批量写、撤销、教师冲突硬塞进路径 1 反复打补丁。

理由：
1. 用户要的是「编排所有房间」，而房间今天就是课表的 owner；任何跨房间操作在路径 1 下都是 N 份课表 + N 次重建 + 无原子性，界面越做到 B/C，后端语义越别扭。
2. 现网课表数据几乎为空（§4：1 份课表、0 模板、0 实例、0 规则、4 条历史事件），是迁移成本最低的窗口；晚一个学期迁移要多处理真实数据与真实事件历史。
3. 事件引擎的引用已经证明脆弱（现网 4/4 事件的 `occurrence_id/schedule_id` 为 NULL），而「稳定 ID」只能靠条目实体实现；路径 2 同时解决重复提醒与断链。
4. 路径 1 的只读聚合（接口 1/2/6）在路径 2 里可以原样保留（JSON 结构对齐），前端不会白做；这也是三条候选界面先拿到真实数据、让用户选定产品形态的最短路径。
5. 路径 2 的迁移以纯新增表 + 兼容列为主，有 `materialize` 开关可回退，风险可控；回填脚本能复用 `expand_occurrences` 做一致性校验。

---

## 4. 事实核查附录（只读探测）

### 4.1 探测方式

- 后端在跑：`GET /healthz` → 200、`GET /readyz` → 200。
- 登录：`POST /api/v1/auth/login`（本地 admin 凭据，见 `frontend/docs/CONVENTIONS.md`，本文不复述）→ 200，拿到 Bearer。
- 只做 GET；未对真库执行任何 POST/PATCH/PUT/DELETE（登录接口除外，且不写业务表）。
- 辅助只读 SQL：`psql -c "SELECT count(*) ..."`，用于确认表的真实行数（无写入）。

### 4.2 探测结果

| 探测 | 结果 |
| --- | --- |
| `GET /admin/areas?page_size=100` | `total=12`，9 个不同名称（E2E 区域、Incus 测试区域、P3 区域、RustFS Smoke、agent config test area×4、在线状态测试、离线通知测试、配置测试区域）→ 全部为开发/测试产物 |
| `GET /admin/rooms?page_size=100` | `total=12`，11 个绑定了 `device_slot`（如 `room-101`、`p3-room`），1 个未绑定（RustFS Room） |
| `GET /admin/schedules?page_size=100` | `total=1`：`name="test"`、`room_name="离线通知房间"`、`2026-10-31 ~ 2026-10-31`、`is_active=true` |
| `GET /admin/schedules/{id}/weekly-slots` | `[]`（该课表没有模板） |
| `GET /admin/schedules/{id}/exceptions` | `[]` |
| `GET /admin/schedules/{id}/occurrences?from=2026-10-25&to=2026-11-05` | `[]`（无物化实例） |
| `GET /admin/rooms/{p3-room}/occurrences?...` | `[]` |
| `GET /admin/rooms/{p3-room}/event-rules` | `[]` |
| `GET /admin/schedule-events?page_size=5` | `total=4`；全部 `room_id=33cf…`（P3 房间）；`occurrence_start_before ×2`、`daily_time ×1`、`occurrence_start ×1` |
| 4 条事件的关联列（psql） | `schedule_id`、`occurrence_id` **全部为 NULL**，但 `payload.occurrence_id` 仍保留旧 UUID → 证明「重建删实例 → 事件引用被置空」 |
| 投递行（psql） | 12 行 = 4 事件 × 3 目标；`downlink 4 delivered`、`pubsub 4 delivered`、`webhook 4 skipped`（房间/区域/全局都没配 webhook） |
| `GET /admin/occurrences?from=2026-11-01&to=2026-11-07` | **404**（确认无跨房间聚合端点） |
| 表行数（psql，只读） | `areas=12, rooms=12, schedules=1, schedule_weekly_slots=0, schedule_exceptions=0, schedule_occurrences=0, schedule_event_rules=0, schedule_events=4, schedule_event_deliveries=12, room_devices=0` |
| `GET /admin/audit-logs?page_size=100` | 能看到 `schedule.create / schedule.delete / schedule.set_weekly_slots / schedule.delete_exception` 等，`detail` 仅摘要（如 `{"count":3}`、`{"date":"2026-11-06"}`） |

### 4.3 探测结论与限制

- 现网**没有可用的真实课表数据**，无法用真实规模验证「500 房间 × 一学期」的性能；本文的性能判断来自代码结构（索引、串行重建、每写必重建）而非压测。**建议在 P1 前用脚本造 200 房间/一学期的影子数据做一次重建耗时基线。**
- 现网历史事件证明事件↔实例的引用会断（4/4），这与路径 2 的「稳定 ID」诉求直接相关。
- 未探测：Redis 锁竞争、多副本并发、Webhook 实际超时行为（需构造数据，超出只读范围）。
- 文档债务：`crates/domain/src/schedule.rs:6` 注释仍称「不引入 chrono-tz」，与实现不符；`occurrences_starting/ending`（`db:1482,1498`）无调用方。

---

## 5. 待用户决策的问题（每条附默认建议）

| # | 问题 | 默认建议 |
| --- | --- | --- |
| 1 | 课程 / 教师 / 班级是否要独立实体（可复用、可改名、可查教师冲突）？ | **课程 + 教师 + 班级都建独立实体**（轻量：code/name/颜色/启用），条目同时存名称快照，避免主数据改名破坏历史 |
| 2 | 是否需要「学期 + 发布版本」概念（草稿不影响设备，可回滚）？ | **需要**：`terms` + `schedule_plans(draft/published)` + 发布快照回滚；这是「撤销」和「一次编排一学期」的前提 |
| 3 | 跨房间冲突（同教师同时段、同房间重叠）是否必须拦截？ | **发布时硬拦截**同房间/同教师冲突（409，可 `force` 并记审计）；草稿只返回警告；班级冲突先做警告（班级可能不完整） |
| 4 | 一次编排的粒度：学期 / 周 / 单日？ | **学期为主线**（plan 绑定 term），周模板式编辑 + 条目级单日调整两级；周视图只是渲染，不单独存实体 |
| 5 | 是否保留现有 per-room 课表页（SchedulesView / ScheduleDetailView / RoomSchedulesPanel）？ | **保留为只读兼容视图一个版本周期**，读时由 plan 投影；写入口收敛到新「排课工作台」 |
| 6 | 设备联动（开课前下发指令、停课/改课通知设备）是否属于本次范围？ | **不属于本次重构范围，但要保证兼容**：沿用现有 `schedule_event_rules`（房间维度）与投递链路；新增「改课/停课」事件类型放到下一期 |
| 7 | 一门课占多个房间（合班/分组/连堂）如何表达？ | **一期一条条目一个房间**；多房间 = 多条条目共享 teacher/class；`plan_entry_rooms` 与连堂（`periods`）放到路径 2 的 P3 之后 |
| 8 | 权限是否要按区域 / 教师收敛（例如教务只编辑自己区域）？ | **本期沿用全局 `schedule:*`，新增 `plan:publish` 收敛发布权**；区域/教师级授权作为独立议题，下一期做 |
