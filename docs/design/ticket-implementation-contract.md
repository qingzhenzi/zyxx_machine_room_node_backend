# 工单系统实现契约（v1.1 / P0）

> 状态：**已实现（v1.1 / P0，2026-11-01）**。本文件是并行开发的唯一契约，后端 DTO、DB 列、前端类型/方法名以此为准。
> 修订：v1.1（2026-11-01，独立 QA 复验后补充：`updated` 事件（迁移 `0019_ticket_event_updated.sql`）、同源 occurrence 不双计、审计 `from_status` 取旧值、`assign` 必须显式 `assignee_id`（null=释放）、`ticket.update` 审计 action、keyword 通配符转义）。
> 实现位置：后端 `crates/domain/src/ticket.rs`、`crates/db/src/tickets.rs`、`crates/api/src/api/admin/tickets.rs`、`crates/api/src/ticket_worker.rs`、`migrations/0018_tickets.sql` + `migrations/0019_ticket_event_updated.sql`；
> 前端 `frontend/src/views/Ticket*.vue` + `frontend/src/features/tickets/` + `frontend/src/stores/ticketTodo.ts`；
> 端到端与验证见文末 §11（全部通过）。
> 设计背景与取舍见 `docs/design/ticket-system-analysis.md`。
> 范围：P0 = 工单模型/状态机/权限 + 手工建单 + 自动建单（机器健康/设备离线/课表投递失败）+ 列表/详情/分析页 + 顶栏「我的待办」。
> 不做（P1）：SSE、邮件、规则配置页、附件、数据范围隔离、巡检计划。

## 1. 命名与枚举（规范，勿改）

```
status:    open | in_progress | resolved | closed | cancelled
priority:  urgent | high | normal | low
category:  machine_health | device_offline | delivery_failed | distribution_failed | security | other
source:    auto | manual
event kind: created | claimed | assigned | status_changed | priority_changed | commented | occurred | updated
```

- 工单主键 `id`（UUID）；人类可读 `code = T-YYYYMMDD-####`（按创建日流水，`tickets_code_seq` 全局序列）。
- 时间全部 RFC3339 UTC（`snake_case` 字段名）。
- 审计 action：`ticket.create / ticket.assign / ticket.status / ticket.comment / ticket.update`；`resource_type = "ticket"`。

## 2. 数据库（`migrations/0018_tickets.sql`）

见设计文档 §4.5 的 DDL，必须包含：

- `tickets`：`id, code, title, description, status, priority, category, source, source_type, source_id, dedupe_key, area_id, room_id, serial, device_slot, reporter_id, assignee_id, due_at, acknowledged_at, resolved_at, closed_at, resolution, occurrence_count, last_seen_at, metadata, created_at, updated_at`。
- 部分唯一索引 `tickets_dedupe_active (dedupe_key) WHERE dedupe_key IS NOT NULL AND status IN ('open','in_progress')`。
- `ticket_events`：`id BIGSERIAL, ticket_id, kind, actor_id, from_status, to_status, note, payload, source_type, source_id, created_at`；`(ticket_id, source_type, source_id) WHERE kind='occurred'` 部分唯一。
- 权限播种：`ticket:read/create/handle/manage`；admin 全量；`operator` 角色（`is_system = FALSE`）= ticket:read/create/handle + device:read + area:read + room:read；viewer/editor 只读。
- `updated_at` 用 `set_updated_at()` 触发器。

## 3. 后端域模型（`crates/domain/src/ticket.rs`）

- `TicketStatus / TicketPriority / TicketCategory / TicketSource / TicketEventKind`：`FromStr`/`as_str`，非法值 400。
- `Ticket`（Serialize，字段同 DB，另带 `reporter_name/assignee_name` 便于前端）。
- `TicketEvent`（Serialize，带 `actor_name`，系统事件为 `"系统"`）。
- `TicketContext`（详情关联上下文，全部可空）：`room_id, room_name, room_code, area_id, area_name, serial, device_slot, health_status, health_score, collected_at, online`。
  - `health_status/health_score/collected_at` 取该 serial 最新 `machine_snapshots`；
  - `online` = device-pki 会话在线或最近 5 分钟内有快照；无法判断时 `null`。
- `TicketDetail { ticket, events, context: TicketContext }`。
- `TicketSummary { by_status: BTreeMap<String,i64>, unassigned, overdue, mine_active }`。
- `TicketStats`：`from,to,totals{created,resolved,closed,open_now,overdue,avg_resolve_secs,median_resolve_secs},by_day[{date,created,resolved,closed}],by_category[{category,count}],by_priority[{priority,count}],top_rooms[{room_id,room_name,count}],top_devices[{serial,device_slot,count}],by_assignee[{assignee_id,display_name,active,resolved,avg_resolve_secs}]`。
- `TicketFilter`（list 参数）、`NewTicket`（建单）、`TransitionInput`。
- 状态机纯函数：`can_transition(from,to) -> bool`；`claim` 只允许 open→in_progress；`resolve` 必须带 resolution；`closed` 前必须 resolved；`reopen` 允许 resolved/closed→open（清空 resolved_at/closed_at）。
- 单测覆盖状态机、优先级 SLA（urgent 1h/high 4h/normal 24h/low 72h）。

## 4. RBAC

`crates/domain/src/rbac.rs`：新增 4 常量并追加 `ALL`；测试断言 `ALL.len()` 21→25，并断言 4 个 ticket 权限在 ALL 中。

## 5. DB 仓储（`crates/db/src/tickets.rs`）

```rust
insert_ticket(&NewTicket) -> Ticket            // 内部 ON CONFLICT dedupe DO NOTHING；返回冲突时 bump occurrence
list(&TicketFilter) -> Page<Ticket>             // 含 reporter_name/assignee_name（LEFT JOIN users）
get(id) -> (Ticket, Vec<TicketEvent>)           // 事件按 id ASC
update_content(id, title?, description?, priority?, category?) -> Ticket
claim(id, user_id, now) -> Ticket
assign(id, Option<user_id>, now) -> Ticket
transition(id, input, actor_id, now) -> Ticket
add_comment(id, actor_id, body) -> TicketEvent
summary(user_id) -> TicketSummary
stats(from, to) -> TicketStats
assignees() -> Vec<TicketAssignee>             // 拥有 ticket:handle 的用户
context(ticket) -> TicketContext               // room_name/room_code、machine 最新 health_status/online（可空）
bump_occurrence(dedupe_key, source_type, source_id, payload) -> Option<Ticket>
```

- 所有写操作事务内同时写 `ticket_events`；状态机冲突返回 `DbError::Conflict` → 409。
- 列表分页沿用 `Page<T> { items,total,page,page_size }`；`page_size` 默认 20、上限 100。
- `mine`：`assignee_id = 当前用户`；`active`：`status IN ('open','in_progress')`；`overdue`：`due_at < now() AND status IN ('open','in_progress')`；`unassigned`：`assignee_id IS NULL`。

## 6. API（`crates/api/src/api/admin/tickets.rs`，挂在 `/api/v1/admin`）

| 方法 | 路径 | 权限 | Body/Query | 响应 |
| --- | --- | --- | --- | --- |
| GET | `/tickets` | ticket:read | status, category, priority, source, assignee_id, unassigned, mine, active, overdue, area_id, room_id, serial, keyword, from, to, page, page_size | `TicketPage` |
| GET | `/tickets/summary` | ticket:read | — | `TicketSummary` |
| GET | `/tickets/stats` | ticket:read | from, to（默认近 30 天） | `TicketStats` |
| GET | `/tickets/assignees` | ticket:handle | — | `TicketAssignee[]` |
| POST | `/tickets` | ticket:create | `{title,description?,category?,priority?,room_id?,area_id?,serial?,device_slot?}` | `201 Ticket` |
| GET | `/tickets/{id}` | ticket:read | — | `TicketDetail` |
| PATCH | `/tickets/{id}` | ticket:manage | `{title?,description?,priority?,category?}`（改动写 `updated` / `priority_changed` 事件；审计 `ticket.update`） | `Ticket` |
| POST | `/tickets/{id}/claim` | ticket:handle | — | `Ticket` |
| POST | `/tickets/{id}/assign` | ticket:manage | `{assignee_id: uuid|null}`（字段必填；显式 null = 释放） | `Ticket` |
| POST | `/tickets/{id}/transition` | ticket:handle | `{to_status,resolution?,note?}` | `Ticket` |
| POST | `/tickets/{id}/comments` | ticket:handle | `{body}` | `201 TicketEvent` |

- 静态路径 `/tickets/summary`、`/tickets/stats`、`/tickets/assignees` 必须注册在 `/tickets/{id}` 之前（axum matchit 静态优先，但保持可读顺序）。
- 状态机/认领冲突 → 409；目标转派人无 ticket:handle → 400；无权限 → 403。
- `transition` 到 `in_progress`：无处理人 → 设当前用户；已分配给自己 → 幂等；已分配他人 → 409（与 claim 语义一致）。
- 自动单重复发生计数以“源行”为准：同一 `(ticket_id, source_type, source_id)` 只累加一次（created 事件与 occurred 事件不得重复计数），不同源行才 `occurrence_count + 1`。
- 人工写操作写审计（`record_or_log`）；PATCH 内容修改用 `ticket.update`，状态流转/认领用 `ticket.status`，转派用 `ticket.assign`，评论用 `ticket.comment`。

## 7. 自动建单 worker（`crates/api/src/ticket_worker.rs`）

- `spawn(state) -> JoinHandle`，`lib.rs` 在 `schedule_engine_enabled` 旁按 `config.tickets_auto_enabled` 启动；`interval(config.ticket_tick_secs)` + `MissedTickBehavior::Delay`；单轮错误只 log。
- 首轮只建基线（记录各 serial 的 health 连续计数），不重放历史。
- 规则：
  1. machine_health：每 serial 最新连续 `ticket_health_consecutive`（默认 2）条 `health_status != 'ok'` → dedupe `machine_health:{serial}`，critical→high，warning→normal，category machine_health，source_type `machine_snapshot`，`source_id` 最新快照 id，metadata 存 `{health_status,health_score,issues}`。
  2. device_offline：`device_session` 离线且 `updated_at <= now() - offline_minutes`（默认 10），房间/区域启用 → dedupe `device_offline:{serial}`，normal，source_type `device_presence`。
  3. schedule_delivery：near lookback（默认 24h）内 `schedule_event_deliveries.status='failed'`，按 `room_id + target + 本地日期` 聚合 → dedupe `schedule_delivery:{room}:{target}:{date}`，high，source_type `schedule_delivery`，`source_id` = event_id。
- 去重用 insert 冲突 → `bump_occurrence`（occurrence_count+1、last_seen_at、追加 occurred 事件）。
- 配置（`crates/api/src/config.rs`）：`TICKETS_AUTO_ENABLED`（默认 true）、`TICKET_TICK_SECONDS`（默认 60，10..=600）、`TICKET_DEVICE_OFFLINE_MINUTES`（默认 10，1..=1440）、`TICKET_HEALTH_CONSECUTIVE`（默认 2，1..=10）、`TICKET_LOOKBACK_HOURS`（默认 24，1..=168）。`.env.example` 同步补注释。

## 8. OpenAPI

- `crates/api/src/api/openapi.rs`：新增 tag `Tickets`、components/schemas（`Ticket`、`TicketPage`、`TicketDetail`、`TicketEvent`、`TicketSummary`、`TicketStats`、`TicketAssignee`、`CreateTicketRequest`、`UpdateTicketRequest`、`AssignTicketRequest`、`TransitionTicketRequest`、`CommentTicketRequest` 及各枚举）、11 条 paths；更新 `spec_lists_core_paths_and_security` 断言。
- 新增 `crates/api/examples/dump_openapi.rs`（`fn main(){ println!("{}", serde_json::to_string_pretty(&qe_api::api::openapi::json_spec())...); }`），供离线导出 spec。

## 9. 前端（`frontend/`）

### 9.1 权限/路由/导航（Lead 负责）

- `src/utils/permissions.ts`：`ticketRead/ticketCreate/ticketHandle/ticketManage`。
- `src/features/roles/permissionGroups.ts`：`ticket: '工单'` + 顺序。
- `src/components/ui/AppIcon.vue`：`ticket`、`chart` 两个线性图标（参考 `prototype/tickets/index.html` sprite）。
- `src/router/routes.ts`：`/tickets`（name `tickets`）、`/tickets/insights`（`ticket-insights`，activeName `tickets`）、`/tickets/:id`（`ticket-detail`，activeName `tickets`），静态 insights 在 `:id` 前，权限 `ticketRead`。
- `src/components/layout/AppSidebar.vue`：「资源」后新增「运维」分组（工单、工单分析，`ticketRead`）。
- `src/components/layout/NotificationsMenu.vue`：「我的待办工单」分组（新 store，独立已读键 `rn:tickets:seen-at`，60s 刷新，点击 `router.push({name:'ticket-detail',params:{id}})`），徽标 = presence unread + ticket unread；不改 presence 逻辑。
- `src/stores/ticketTodo.ts`：`refresh()` 调 `api.listTickets({ mine:true, active:true, page_size:10 })`；无 `ticketRead` 或页面隐藏时跳过；错误静默。

### 9.2 API 客户端（Lead 负责）

- `src/api/types.ts`：`TicketListParams`（含 mine/active/overdue）、`TicketStatsParams`。
- `src/api/index.ts` 方法：
  `listTickets(params) -> TicketPage`、`ticketSummary() -> TicketSummary`、`ticketStats(params) -> TicketStats`、`ticketAssignees() -> TicketAssignee[]`、`getTicket(id) -> TicketDetail`、`createTicket(input) -> Ticket`、`updateTicket(id,input) -> Ticket`、`claimTicket(id) -> Ticket`、`assignTicket(id,input) -> Ticket`、`transitionTicket(id,input) -> Ticket`、`commentTicket(id,input) -> TicketEvent`。
- 类型来自再生成的 `src/api/schema.ts`（由 OpenAPI 生成，禁止手写响应类型）。

### 9.3 页面（frontend-pages-dev 负责）

- `views/TicketsView.vue`：PageHeader（新建工单）、KPI 条（待处理/处理中/已解决/超时，来自 `ticketSummary`）、状态 tabs（带计数）、筛选（优先级/分类/处理人/搜索/时间/我的）、表格（编号/标题/状态/优先级/分类/对象/来源/处理人/更新时间）、分页 20、三态、URL query 同步、行点击进详情、`AppModal` 新建。
- `views/TicketDetailView.vue`：返回链接、页头（code+title+状态/优先级/分类/来源胶囊+按状态操作按钮）、问题描述、触发信号、右栏工单信息 + 关联上下文（房间/设备/机器健康与在线 + 跳转）、时间线、评论框、重开/取消确认；403/404 分开提示。
- `views/TicketInsightsView.vue`：近 7/30/90 天、KPI（新增/已解决/未关闭/MTTR/超时）、纯 CSS 新增 vs 关闭柱状图、分类分布、房间/设备 TOP5、处理人负载、自动洞察文案。
- `features/tickets/`：`ticketMeta.ts`（文案/tone/`availableActions(status,canHandle)`）、`TicketFilters.vue`、`TicketTable.vue`、`TicketTimeline.vue`、`TicketContextCard.vue`、`TicketCreateModal.vue`、`TicketStatsCards.vue`、`TicketTrendChart.vue`、`TicketCategoryBars.vue`、`ticketStats.ts`（纯函数：按日序列/TOP/超时）。
- 约定：中文文案、无新依赖、图表纯 CSS、禁止 `v-html`、危险操作 `ConfirmDialog`、时间用 `utils/format`、403 用 `isForbidden`。
- 测试：`features/tickets/__tests__/ticketMeta.spec.ts`、`ticketStats.spec.ts`；`e2e/tickets.spec.ts`（登录→列表→新建→详情→评论→认领/解决）。

## 10. 文件归属（并行写入范围）

| 负责人 | 允许修改 |
| --- | --- |
| backend-dev | `migrations/0018_tickets.sql`、`crates/domain/src/{ticket.rs,lib.rs,rbac.rs}`、`crates/db/src/{tickets.rs,lib.rs}`、`crates/api/src/{state.rs,lib.rs,config.rs,ticket_worker.rs}`、`crates/api/src/api/{mod.rs,admin/mod.rs,admin/tickets.rs,openapi.rs}`、`crates/api/examples/dump_openapi.rs`、`crates/api/tests/tickets.rs`、`.env.example` |
| frontend-pages-dev | `frontend/src/views/{TicketsView.vue,TicketDetailView.vue,TicketInsightsView.vue}`、`frontend/src/features/tickets/**`、`frontend/e2e/tickets.spec.ts` |
| Lead | `frontend/src/utils/permissions.ts`、`frontend/src/router/routes.ts`、`frontend/src/components/layout/{AppSidebar.vue,NotificationsMenu.vue}`、`frontend/src/components/ui/AppIcon.vue`、`frontend/src/features/roles/permissionGroups.ts`、`frontend/src/stores/ticketTodo.ts`、`frontend/src/api/{index.ts,types.ts,schema.ts}`、`frontend/docs/openapi.json`、文档 |

## 11. 验收命令

```bash
cargo fmt -p qe-api -p qe-domain -p qe-db
cargo check -p qe-api
cargo test -p qe-domain
cargo test -p qe-api --test tickets -- --ignored   # 需要 PG/Redis；环境不具备则说明
cargo run -p qe-api --example dump_openapi > frontend/docs/openapi.json
cd frontend && python3 docs/openapi-types.py && ./node_modules/.bin/oxfmt src/api/schema.ts
./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json
./node_modules/.bin/vitest run
./node_modules/.bin/vite build
```

前端页面必须能在 `vue-tsc --noEmit` 下通过；后端必须 `cargo check` 通过；有 DB 条件时跑集成测试。

### 11.1 落地验证记录（2026-11-01，全部通过）

| 检查 | 命令 | 结果 |
| --- | --- | --- |
| 后端编译 | `cargo check -p qe-api` | ✅ |
| 领域单测 | `cargo test -p qe-domain` | ✅ 87 passed（状态机/SLA/枚举） |
| 仓储单测 | `cargo test -p qe-db` | ✅ 20 passed（含 `escape_like`） |
| API 单测 | `cargo test -p qe-api --lib` | ✅ 61 passed（含 OpenAPI 断言、worker 规划） |
| 工单集成（真库） | `TEST_DATABASE_URL=…/qestudio_test cargo test -p qe-api --test tickets -- --ignored --test-threads=1` | ✅ 3 passed（权限/状态机/去重/多副本同源计数/三条规则/审计/context） |
| OpenAPI 导出 | `cargo run -q -p qe-api --example dump_openapi` | ✅ 79 paths / 25 个 Ticket* schema（含 `updated` 事件与 assign required） |
| 前端类型 | `vue-tsc --build` | ✅ 0 errors |
| 前端单测 | `vitest run` | ✅ 92 passed（工单 48：meta/stats/组件挂载/设置弹窗） |
| 前端构建 | `vite build` | ✅ |
| 前端 lint | `oxfmt --check src` + `oxlint src` | ✅ 0 warnings / 0 errors |
| 端到端 | `E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 playwright test e2e/tickets.spec.ts --project=chromium` | ✅ 3 passed（新建→评论→认领→设置→解决→关闭→列表检索 / 分析页 / URL 同步） |

真实页面截图：`docs/design/ticket-implementation-{list,detail,insights,notification}.png`。
独立 QA 报告：`docs/design/_tickets-qa-report.md`。
