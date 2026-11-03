# 工单系统 P0 独立 QA 验证报告

> 验证人：qa-verifier（独立 QA，未参与实现）｜时间：2026-11-01｜
> 对象：`room_node_backend` 工单系统 P0（后端 axum + 前端 Vue3 控制台）
> 依据：`docs/design/ticket-implementation-contract.md`（冻结契约）、`docs/design/ticket-system-analysis.md`
> 方式：只读代码审查 + 针对性命令 + 真实 PG/Redis 集成测试 + 运行中栈（API `127.0.0.1:8081`、Vite `127.0.0.1:5175`）的 API/UI/Playwright 实测。
> 本次验证只写本文件，未修改任何源码；创建的临时数据已清理（详见 §8）。

---

## 1. 结论（verdict）

> **复验更新（2026-11-01 20:05，第二轮定向复验）**：原报告的 P1-1/P1-2/P1-3、P2-4/6/7/8/9/10/12 及 P3 日志/seen_deliveries 均已修复并通过复验；前端已补「工单设置」（转派/优先级/分类）入口与 `updated` 事件文案。**最终 verdict：PASS**（无 release blocker；剩余 P3 为已确认的产品取舍/刻意行为/在途改动，见 §11）。
> 以下 §1–§10 为**首轮**验证的原始结论与问题记录，保留作历史基线；各问题的最新状态以 §11 复验结果为准。

**PASS-with-issues（首轮结论，已被 §11 复验取代）** —— **未发现 release blocker**。主链路（建单→列表/详情→认领→解决→关闭/重开/取消、权限、分页、状态机 409/400、自动建单三条规则、去重、统计、前端 3 页 + 通知分组、e2e）均可正常工作，实测与契约基本一致；但存在 **2 个中危契约/语义偏差 + 1 个审计信息错误**（建议发布前修复），另有若干低危问题与工作区/合并范围风险。

问题严重度排序（P1 = 中危，建议修；P2 = 低危/建议改进；P3 = 观察项）：

| # | 严重度 | 问题 | 位置 |
|---|--------|------|------|
| 1 | P1 | `PATCH /tickets/{id}` 仅改标题/描述时**不写 `ticket_events`**，违反契约 §5「所有写操作事务内同时写 ticket_events」 | `crates/db/src/tickets.rs:445-515`；`crates/api/src/api/admin/tickets.rs:431-457` |
| 2 | P1 | 自动单 **`occurrence_count` 对同一源行重复计数**（created 事件与 occurred 事件用同一 `source_id` 各计一次）→ 单次信号即显示「重复发生 2 次」；多副本下同样发生（已实测） | `crates/db/src/tickets.rs:370-424,1453-1493` |
| 3 | P1 | 状态流转审计 detail 的 `from_status` **记录成流转后的新状态**（应为旧状态） | `crates/api/src/api/admin/tickets.rs:542-555` |
| 4 | P2 | 自动建单 `schedule_delivery` 的 `source_id` 用了 **delivery_id**，契约 §7 要求 **event_id** | `crates/api/src/ticket_worker.rs:416` |
| 5 | P2 | 前端**没有任何「转派 / 改优先级 / 改分类」入口**（`assignTicket`/`updateTicket` 已封装但无 UI 调用），分析文档 §10.1 验收标准 2「admin 能转派、改优先级」在 UI 层不可达 | `frontend/src/views/TicketDetailView.vue`、`frontend/src/features/tickets/TicketTable.vue` |
| 6 | P2 | `transition{to_status:"in_progress"}` 对已转派给他人的 open 工单保留旧 `assignee`（`COALESCE`），与设计 §4.2「认领/开始处理设 assignee=自己」不一致；他人可「开始处理」同事的工单 | `crates/db/src/tickets.rs:657-669` |
| 7 | P2 | `POST /tickets/{id}/assign` 空 body `{}` **静默释放处理人**（OpenAPI 标 `assignee_id` required；契约 body 必填），易误操作 | `crates/api/src/api/admin/tickets.rs:134-138,484-508` |
| 8 | P2 | PATCH 内容修改写审计 action = `ticket.status`（契约仅 4 个 action，语义不符），且与 `ticket.create/assign/comment` 混在状态类动作里 | `crates/api/src/api/admin/tickets.rs:443-457` |
| 9 | P2 | `oxfmt --check src` 失败：`frontend/src/api/index.ts` 未按项目格式格式化 | `frontend/src/api/index.ts` |
| 10 | P3 | `keyword` 未转义 ILIKE 通配符：`keyword=%` 会匹配全部工单（非注入，属搜索语义瑕疵） | `crates/db/src/tickets.rs:69-71` |
| 11 | P3 | worker `seen_deliveries` 只增不减（长运行内存缓慢增长）；且「自动工单已创建/累加」日志把「仅 bump/无变化」也计入 `created` 并打 info（日志误导） | `crates/api/src/ticket_worker.rs:51,123-137,181-195,250-289` |
| 12 | P3 | OpenAPI 描述与实现不符：`mine` 描述写「只看分配给我的未结束工单」，实现只按 `assignee=me`（是否 active 由 `active` 参数单独控制）；`assign` 的 requestBody 标 required 但空 body 可成功 | `crates/api/src/api/openapi.rs:2876-2890` |
| 13 | P3 | 工单进入 resolved/closed/cancelled 后详情页隐藏评论框（后端允许评论）；契约未禁止，属产品取舍 | `frontend/src/views/TicketDetailView.vue:73-75` |
| 14 | P3 | 工作区混入**无关的排课工作台（schedule-board）在途功能**，且与工单改动同文件耦合（`openapi.rs`/`routes.ts`/`api/index.ts`/`openapi.json`），工单变更无法单独合并；另有 6 个前端临时脚本未清理 | `git status`（见 §7） |
| 15 | P3 | `bootstrap.rs` 改为**每次进程启动都 bump 权限代际**（契约未要求；为规避 5 分钟权限缓存 403 的合理做法，但属全员行为变更，发布说明需知会） | `crates/api/src/bootstrap.rs:12-24` |

---

## 2. 环境与验证范围

- 后端：`./target/debug/qe-api`（`BIND_ADDR=127.0.0.1:8081 RUN_MIGRATIONS=true`），migration `0018_tickets` 已应用（`_sqlx_migrations.version=18`，`success=t`）。
- 另有一个 8080 副本进程同时运行（**实测形成双副本**，用于观察多副本去重）。
- Vite：`VITE_BACKEND_ORIGIN=http://127.0.0.1:8081 vite --port 5175`；登录 `admin / Admin@12345`。
- DB：docker PostgreSQL `qestudio`（dev）/`qestudio_test`（集成测试）；Redis 6379。
- 未重启/未 kill 任何服务，未改源码，未跑 pnpm install、无外网请求。

---

## 3. 验收命令与结果（逐条实测）

| # | 命令 | 结果 |
|---|------|------|
| 1 | `cargo check -p qe-api` | ✅ `Finished dev profile`（0.21s，命中缓存）；随后 `cargo test -p qe-api --no-run` 全量编译测试目标成功（1m13s），等价证明可编译 |
| 2 | `cargo test -p qe-domain` | ✅ 87 passed; 0 failed（含 ticket 枚举/状态机/SLA/单号 11 个用例） |
| 3 | `cargo test -p qe-api --lib` | ✅ 61 passed; 0 failed（含 `ticket_worker` 首轮基线/三条规则规划、`openapi` 路径断言、router 无冲突） |
| 4 | `cargo test -p qe-api --test tickets -- --ignored --test-threads=1`（`TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test TEST_REDIS_URL=redis://127.0.0.1:6379`） | ✅ 3 passed; 0 failed; 8.72s（生命周期+权限、去重/occurrence、worker 候选查询+首轮不重放） |
| 5 | `vue-tsc --noEmit -p tsconfig.app.json` | ✅ exit 0 |
| 6 | `vitest run` | ✅ 7 files / 91 tests passed（ticketMeta 16、ticketStats 18、TicketComponents 13，其余为既有测试，无回归） |
| 7 | `vite build` | ✅ 成功 1.20s；`TicketsView/TicketDetailView/TicketInsightsView` chunk 均构建 |
| 8 | `oxlint src` | ✅ 0 warnings / 0 errors（174 files） |
| 9 | `E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 playwright test e2e/tickets.spec.ts --project=chromium --reporter=list` | ✅ 3 passed（6.4s）：新建→评论→认领→解决→关闭→列表检索；分析页；列表筛选 URL query 同步+刷新恢复 |
| 10 | `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | ✅ exit 0 |
| 11 | `oxfmt --check src` | ❌ 1 file：`src/api/index.ts`（见问题 #9） |
| 12 | 迁移/索引/触发器（psql） | ✅ `tickets_dedupe_active` 部分唯一索引、`ticket_events_occurred_key` 部分唯一索引、7 个查询索引、`tickets_set_updated_at` 触发器均存在且生效（事务内验证唯一起作用、跨事务 UPDATE 触发更新时间） |
| 13 | 生成物一致性 | ✅ `curl /api-docs/openapi.json` 与 `frontend/docs/openapi.json` canonical JSON **完全一致**；用 `docs/openapi-types.py` 重新生成 + 项目 `.oxfmtrc.json` 格式化后与 `frontend/src/api/schema.ts` **逐字节一致**（生成链路无漂移） |
| 14 | 运行态 UI 冒烟（Playwright 脚本，非仓库文件） | ✅ `/tickets` 4 个 KPI、12 行数据、筛选器存在；通知菜单出现「我的待办工单」分组（1 项，admin 在办单）；`/tickets/insights` 四块图表存在；登出访问 `/tickets` → `/login?redirect=/tickets`；**ticket 页面 console error / pageerror 均为 0** |

---

## 4. 契约一致性 / 行为实测（运行中栈，节选关键用例）

全部用真实 API（admin + 临时 operator×2 + 无 ticket 权限用户）与真实 DB 校验：

### 4.1 权限（契约 §6）
- 未认证 `GET /admin/tickets` → **401**（`缺少 Authorization 请求头`）。
- 无 `ticket:read` 用户 list/summary/stats/detail → **403**；无 `ticket:create` 建单 → **403**；无 `ticket:handle` 评论/认领/assignees → **403**；operator（仅 handle）PATCH/assign → **403 缺少 ticket:manage**。
- `GET /tickets/assignees`（operator，有 handle）→ 200，返回 admin + 2 个 operator（`roles` 字段正确）。
- 角色播种正确：`admin` 25 项权限含 4 个 ticket；`operator` = `ticket:read/create/handle + device:read/area:read/room:read`（**无 manage**）；`viewer/editor` 仅 `ticket:read`。
- admin 启动补齐：登录响应 permissions 已含 `ticket:*`（迁移+bootstrap 均生效）。

### 4.2 列表 / 分页 / 参数（契约 §5/§6）
- 分页结构 = `{items,total,page,page_size}`，`page_size=2` 时 items=2、total 正确；`ORDER BY updated_at DESC` 正确。
- 非法 `status/priority` → 400（中文域错误）；`page_size=101` → 400；`page=0` → 400；`page=9223372036854775807` → 400 保护（无溢出 500）；`keyword>100 字符` → 400；非法 UUID query → 400（沿用既有纯文本 rejection，与其它端点一致）。
- `mine=true&active=true`、`unassigned=true&active=true`、`overdue=true`、`source=auto`、`serial=`、`from/to`（无数据时 total=0）、`assignee_id + unassigned` 优先级（assignee_id 胜出）均实测符合契约/注释。
- 超时判定实测：将一张 QA 单 `due_at` 改为 2h 前 → `list?overdue=true` total=1、`summary.overdue=1`、`stats.totals.overdue=1`，与 SQL 真相一致。
- `get` 详情结构 = `{ticket, events, context}`；`context` 对带 serial 的工单返回 `health_status/health_score/collected_at/online`（实测 warning/58/online=true）；无关联时全 null。不存在 UUID → 404；非法 UUID 路径 → 400。

### 4.3 人工写 / 状态机（契约 §3/§6）
- 建单 201：`code=T-YYYYMMDD-####`、`status=open`、`source=manual`、`reporter_name` 正确、`due_at` = 创建时间 + 优先级 SLA（urgent 1h 实测）；空标题 400；标题 201 字 400；`device_slot` 非法字符 400；room 不存在/area 不存在 404；room 与显式 area 不匹配 400；room 存在时自动带出 area_id。
- `claim`：open→in_progress、`assignee_id=自己`、`acknowledged_at` 写入；他人重复认领 **409**；自己重复认领幂等 200（不重复写事件，仅多写一条审计）。
- `transition`：`in_progress→resolved` 缺/空白 resolution **400**；`in_progress→closed` **409**；`resolved→closed` 200（closed_at 写入）；`closed→in_progress` **409**；`closed→open` 200 且 `resolved_at/closed_at` 清空；`open→cancelled` 200；`cancelled→open` **409**；非法 `to_status` 400。
- `comments` 201 返回 `TicketEvent{kind:"commented",actor_name}`；空评论 400；不存在工单 404；时间线顺序与内容正确（created/priority_changed/claimed/status_changed/assigned/commented）。
- PATCH：operator 403 / admin 200；`description:null` 清空生效；priority/category 变化时间线写 `priority_changed`（payload 含 from/to）。
- 审计：`ticket.create/status/assign/comment` 均有行（`resource_type="ticket"`、`resource_id=ticket.id`）；**自动单 0 条审计**（符合设计 §4.3）。

### 4.4 自动建单 worker（契约 §7）
- 运行日志：8081 启动 `worker 已启动 tick_secs=60 ...`；首轮 `已建立基线（不重放历史） health_suppressed=1 offline_baseline=6 failed_deliveries=0`。
- **首轮不重放实测**：启动时已异常的 serial（`4f40...`）与 6 台已离线设备在之后 10+ 分钟持续异常，但 DB 中**没有**对应自动单。
- **新异常创建实测**（基线之后注入）：新 serial 连续 2 条 warning 快照 → 约 39s 后自动建单 `T-20261101-0013`（`source=auto`、`category=machine_health`、`priority=normal`、`dedupe_key=machine_health:QA-WORKER-...`、`source_type=machine_snapshot`、metadata 含 `issues`）。
- **设备离线规则实测**：新 enrollment + 绑定启用房间 + `device_session.state=offline` 超 30min → 自动建单 `device_offline:QA-OFFLINE-...`（normal、`source_type=device_presence`）。
- **投递失败规则实测**：`schedule_event_deliveries.status='failed'`（webhook, lookback 内）→ 自动建单 `schedule_delivery:{room_id}:webhook:{本地日期}`（high、`source_type=schedule_delivery`）。本地日期按 `areas.timezone` 生成，正确。
- **多副本去重实测**：两个后端副本（8080/8081）同时跑 worker；上述三条自动单均只有 **1 张**（8080 创建、8081 冲突后 bump），未出现重复工单；`ticket_events` 的 occurred 唯一索引阻止了重复 occurred 事件。DB 层：同一 `dedupe_key` 两张 open/in_progress 被部分唯一索引拒绝；`closed` 后可再开新单（事务内验证通过）。
- **`dedupe_key` 冲突后的 occurrence 语义**：详见问题 #2（同一源行 created+occurred 双计）。

### 4.5 统计（契约 §3/§6/设计 §9）
- `summary`：五种状态全量计数（缺失补 0）、unassigned/overdue/mine_active 与 SQL 真值一致。
- `stats`（自定义 from/to）：`totals.created/resolved/closed/open_now/overdue` 与 DB 计数一致；`sum(by_day.created) == totals.created`；`by_category/by_priority/top_rooms/top_devices/by_assignee` 结构/排序符合契约；`avg_resolve_secs/median_resolve_secs` 有值。
- **跨日时区聚合实测**：`10-31T22:00Z` 创建的工单被归入本地（Asia/Shanghai）`2026-11-01`，非 UTC 日期，符合设计 §9 要求。
- `from>=to` / 非法时间 → 400；缺省 30 天窗口生效。

### 4.6 前端（契约 §9）
- 路由 `/tickets`、`/tickets/insights`、`/tickets/insights` 在 `:id` 之前；`meta.permission=ticketRead`、`activeName` 正确；侧边栏「运维」分组；`permissionGroups` 有 `ticket: '工单'`；4 个权限常量存在。
- 列表：KPI、状态 tabs（计数）、筛选（优先级/分类/处理人/我的/时间/搜索）、表格 9 列、分页 20、三态、URL query 同步、行点击进详情、AppModal 新建（实测创建后跳详情）。
- 详情：状态/优先级/分类/来源胶囊、按状态操作按钮（开始/认领/取消/解决/关闭/重开）、解决说明必填、确认弹窗、时间线、评论框、关联上下文、metadata JsonBlock、403/404 分开文案；实测 API 403/409 有 toast 提示。
- 分析页：7/30/90 天、KPI、纯 CSS 趋势图、分类分布、房间/设备 TOP5、处理人负载、自动洞察；实测四块图表渲染。
- **XSS**：全仓 `v-html` 仅出现在注释/既有公告文案提示中；工单页无 `v-html`。实测创建 `<img onerror>` / `<script>` 标题+描述后打开详情页：页面按纯文本插值显示、`window.__xss` 未被设置、无 dialog、无 console error —— **未发现 XSS**。
- 通知中心：「我的待办工单」独立分组 + `rn:tickets:seen-at` 独立已读键 + 60s 独立轮询；实测分组显示（admin 在办单 1 项）、点击跳详情；presence 逻辑未被改动（既有测试通过）。

---

## 5. 问题详情、影响与修复建议

### #1（P1）PATCH 仅改标题/描述不写时间线事件
- 契约 §5 明确「所有写操作事务内同时写 `ticket_events`」；设计 §4.2「改标题/描述/优先级/分类 → 记 `priority_changed` 等事件」。
- 实测：`PATCH {title}` 返回 200，但详情 `events` 仍只有 `["created"]`（无任何事件）；`PATCH {description:null}` 同样无事件。仅 priority/category 变化时才写 `priority_changed`（`crates/db/src/tickets.rs:489-508` 的 `if priority_changed || category_changed` 分支）。
- 影响：详情时间线缺失内容修改轨迹，与「可追溯」目标不一致；审计有记录但审计不等于时间线。
- 修复：`update_content` 在事务内**始终**插入事件（可复用 `priority_changed`，payload 增加 `title_from/title_to/description_changed` 等字段；如需更精确语义需契约新增 kind，但契约冻结情况下复用 + payload 是成本最低方案）。

### #2（P1）自动单 occurrence_count 同一源行双计
- 建单时写 `created` 事件（含 `source_id`），`bump_occurrence` 的 occurred 部分唯一索引只覆盖 `kind='occurred'`，因此**同一 `source_id` 会再追加一条 occurred 并把 `occurrence_count` +1**。实测单信号自动单 `T-20261101-0013`：created(source 2190) + occurred(source 2190)，`occurrence_count=2`；双副本场景（8080 创建、8081 在下一 tick bump 同一 source）同样得到 2。
- 影响：单次告警在 UI 显示「重复发生 2 次」，统计/复盘口径失真；且 machine_health 每次新快照（source_id 变化）都会 bump，若快照频率高于 tick（10s vs 60s），occurrence 会每分钟持续增长（风险提示，当前实测停机数据未触发）。
- 修复（二选一）：
  1. 建单事务内同时向 `ticket_events` 插入一条「创建源行」的 occurred 幂等记录（或把 created 事件纳入 occurred 部分唯一索引的判定：改为 `(ticket_id, source_type, source_id)` 唯一、不限 kind），使同 source 的第一次 bump 冲突返回 None；
  2. `bump_occurrence_tx` 增加 `NOT EXISTS (SELECT 1 FROM ticket_events WHERE ticket_id=... AND source_type=$2 AND source_id=$3)` 前置判断（含 created 事件）。
  另建议 machine_health 的 occurrence 只在「恢复后再次异常」或 health_status 恶化时 +1，而非每个 tick/快照。

### #3（P1）审计 from_status 记成新状态
- `crates/api/src/api/admin/tickets.rs:542-555`：`transition` 完成后用返回的 `ticket.status.as_str()` 作为 `from_status`，而 repo 返回的是流转后的工单。实测审计 detail：`{"action":"transition","to_status":"resolved","from_status":"resolved"}`、`{"to_status":"open","from_status":"open"}` —— from/to 永远相同，信息错误。
- 修复：API 层在调用 transition 前读取旧 ticket（或让 `TicketRepo::transition` 返回 `(old_status, Ticket)`），审计写真实 from_status。claim/assign 的审计同样建议记录旧值（assign 的 previous_assignee 已在事件 payload，但审计缺）。

### #4（P2）schedule_delivery source_id 用 delivery_id
- 契约 §7 第 3 条：`source_type schedule_delivery`，**`source_id` = event_id**；`crates/api/src/ticket_worker.rs:416` 用 `latest.delivery_id.to_string()`。实测自动单 `source_id=13`（delivery id），同一 event 的多个 target 失败会产生不同 source_id 的 bump。
- 修复：`source_id` 改为 `latest.event_id.to_string()`（metadata 里已同时保留 event_id/delivery_id，详情回看不受影响）。

### #5（P2）前端缺转派/改优先级入口
- `frontend/src/api/index.ts` 已封装 `assignTicket/updateTicket/ticketAssignees`，但全前端无任何调用点（grep 仅 `ticketAssignees` 用于筛选下拉）；详情页按钮只覆盖 开始/认领/取消/解决/关闭/重开。
- 冻结契约 §9.3 未显式列转派 UI，故不判为契约违规，但设计 §2.1「可分配：能认领给自己，也能由管理员/负责人转派给别人」与 §10.1 验收标准 2「admin 能转派、改优先级」在 UI 层不可达，建议产品/契约负责人确认是否属于 P0 范围；若属于，补详情页「转派/编辑」弹窗（可复用现成 API）。

### #6（P2）transition 开始处理不覆盖 assignee
- `crates/db/src/tickets.rs:657-669` 用 `assignee_id = COALESCE(assignee_id, $2)`。实测：admin 将 open 工单转派给 op1，op2 调用 `transition{to_status:"in_progress"}` → 200，`status=in_progress` 但 `assignee` 仍为 op1、`acknowledged_at` 记为 op2。与 `claim`（强制覆盖为本人）语义不一致，也不符合设计 §4.2。
- 修复：transition 到 in_progress 时若 `assignee_id` 已存在且 ≠ actor → 409（与 claim 一致），或按设计覆盖为 actor 并记事件 payload。

### #7（P2）assign 空 body 静默释放
- `AssignTicketRequest.assignee_id` 为 `#[serde(default)] Option<Uuid>`，因此 `POST /assign` 空 body 等价 `assignee_id:null`。实测返回 200 且 `assignee_id=null`。OpenAPI 已标 required，契约 body 必填。
- 修复：改为 `assignee_id: Option<Option<Uuid>>`（区分缺失与显式 null），缺失 → 400；或至少前端/文档明确要求显式传 null。

### #8（P2）PATCH 审计 action 语义
- `tickets.rs:443-457` 内容更新写 `ticket.status`。契约审计 action 只有 `ticket.create/assign/status/comment`，其中无 `ticket.update`；建议：要么契约补充 `ticket.update`，要么在 detail 里明确 `action=update_content`（现已如此）但 action 名仍易误导。低优先，需契约负责人决策。

### #9（P2）oxfmt --check 失败
- `frontend/src/api/index.ts`（Lead 负责文件）未按 `.oxfmtrc.json`（semi=false, singleQuote=true）格式化；`oxfmt --check src` 报 1 file。CI 若跑 `format:check` 会红。修复：`cd frontend && ./node_modules/.bin/oxfmt src`。

### #10-#15（P3）
- keyword 通配符：`crates/db/src/tickets.rs:69-71` 用 `ILIKE ('%'||$13||'%')` 未转义 `%/_`；建议对输入做 `replace('\\','\\\\').replace('%','\\%')` 或改用 `position(lower($13) in lower(...))`。
- worker 内存/日志：`TicketWorkerTracker.seen_deliveries` 永久增长；建议按 lookback 窗口定期清理（worker 可返回 `HashSet::retain` 依据本轮候选 + 时间窗）。`created` 计数在 `insert_ticket` 返回 bump/unchanged 时也 +1 并打 info「已创建/累加」，建议 repo 返回枚举（Created/Bumped/Unchanged）或单独的 `inserted` 标志。
- OpenAPI 描述：`mine` 与实现不符；`assign` required 与实现不符（见 #7）。建议改描述 + 实现二选一对齐。
- 详情页终态评论：`canComment` 排除 resolved/closed/cancelled；后端允许。若产品希望"关闭后仍可留言"，去掉 `isTerminalStatus` 限制；否则在契约/文档注明。
- 工作区混入 schedule-board：见 §7。
- bootstrap 每次启动 bump 权限代际：`crates/api/src/bootstrap.rs:12-24`；合理（解决迁移已授权但缓存仍旧导致 403），但会令每次滚动发布使全部副本权限缓存失效一次，建议在发布说明中写明。

---

## 6. 安全 / 回归 / 一致性结论

- **SQL 注入**：`crates/db/src/tickets.rs` 动态 SQL 仅拼接常量（列名/WHERE 模板），所有用户输入均走 `bind`；实测 `keyword=%' OR 1=1--` 返回 200/空结果，无泄漏。**未发现注入**。
- **权限**：11 个 handler 全部有 `require_permission`（逐行核对 + 矩阵实测 403/401）；无缺失鉴权 handler。
- **XSS**：无 `v-html`；纯文本插值；实测脚本不执行（见 §4.6）。
- **审计**：人工写操作均落审计；自动单不写审计；`record_or_log` 失败仅记日志（可接受）。
- **事务一致性**：状态机/认领/转派/评论/建单均在事务内加行锁 + 写事件（`SELECT ... FOR UPDATE`），符合契约；`add_comment` 对已结束工单也允许（契约未禁止）。
- **时区/溢出**：`code` 日期与 `by_day` 均按区域时区；`page` 上限 1_000_000 防溢出；`occurrence_count` 为 INT + 读出转 bigint，极端增长有上限风险（理论值，非现实）。
- **既有功能回归**：`cargo test -p qe-api --no-run` 全测试目标编译通过；`vue-tsc`/`vitest`/`vite build`/既有 `redirect/format/App` 测试均通过；通知中心 presence 逻辑未改（测试通过）；路由守卫/权限工具未破坏。

---

## 7. 工作区范围 / 合并风险（非功能缺陷，但影响发布）

`git status` 显示本次改动**混入了与工单无关的排课工作台（schedule-board）在途功能**，且与工单共享文件：
- 新增/修改：`crates/api/src/api/admin/schedule_board.rs`、`crates/db/src/schedule.rs`、`frontend/src/features/scheduleBoard/**`、`frontend/src/views/ScheduleBoardView.vue`、`docs/schedule-board-redesign.md`、`docs/design/schedule-board-*` 等。
- 共享文件被同时修改：`crates/api/src/api/openapi.rs`（同时含 ScheduleBoard/Tickets spec）、`frontend/src/api/index.ts`（同时含 scheduleBoard 与 tickets 方法）、`frontend/src/router/routes.ts`（schedules 工作台 + tickets 路由）、`frontend/docs/openapi.json`（已包含 schedule-board 路径）。
- 未跟踪的临时脚本：`frontend/.debug-copy.mjs`、`.diag.mjs`、`.smoke-v18.mjs`、`.smoke2.mjs`、`.verify-copy-paste.mjs`、`frontend/.debug-*` 等，建议提交前清理或加入 `.gitignore`。
- 影响：若只合并「工单」变更，`api/index.ts`/`routes.ts`/`openapi.rs` 会因引用 scheduleBoard 而编译失败；工单与排课工作台需要**一起合并或拆分提交**。请 Lead 确认发布范围与提交策略。

---

## 8. 未验证项（明确标注原因，未掩盖）

1. **全仓 `cargo test`（工作区所有 crate 的完整集成测试）**：按任务要求避免数小时全量套件，只跑了任务指定的 `qe-domain`、`qe-api --lib`、`--test tickets --ignored` 与 `--no-run`。
2. **Firefox / WebKit e2e**：只跑任务指定 `--project=chromium`（本地 Playwright 仅系统 Chrome 通道）。
3. **eslint**：按任务清单只跑 `oxlint`；项目另有 eslint 脚本，未运行。
4. **权限缓存 300s / 滚动发布权限代际**：未实测（需等待 5 分钟或构造多副本旧缓存）；bootstrap 已做启动 bump，代码审查通过。
5. **多副本并发压力**：仅用「8080+8081 双副本 + 同一信号」观察去重（结果正确）；>2 副本、同毫秒 tick 的并发压测未做。
6. **前端通知未读角标数值 / markSeen/markAllSeen localStorage 行为**：做了 UI 冒烟（分组+条目+跳转）与代码审查，未做单测级断言（该 store 无现成测试）。
7. **stats 的 MTTR 中位数/cancelled 排除边界、by_assignee 区间口径**：代码审查 + 当前数据验证，未构造边界数据集。
8. **machine_health 的「按槽位映射房间」**：本次自动单的 QA serial 未绑房间（room_id 为空），真实 `room_devices` 映射路径由集成测试 `worker_signal_queries...` 覆盖（配置房间校验通过），未在 dev 环境做端到端。
9. **`TICKETS_AUTO_ENABLED=false` 启动路径**：仅代码审查（配置解析/范围校验），未实际以 false 重启验证（不允许重启服务）。
10. **无权限用户的 UI 403 页面**（路由守卫 + EmptyState）：API 403 已实测，UI 仅代码审查（403 分支存在且文案正确），未用无权限账号走浏览器。
11. **真实浏览器 1280 宽度视觉回归**：未做截图级视觉对比（原型图未逐像素比对）。

---

## 9. 测试数据清理

- 已删除本次 QA 创建的 10 张 QA/e2e 工单（含 worker 自动单 T-0013/0026/0027）、对应 `ticket_events`（级联）、3 个临时用户、1 个临时角色、2 套 worker fixture（机器快照/设备 enrollment/离线 session/排课事件+投递/临时房间）。dev 库恢复为原有 5 张工单。
- 删除的临时用户对应审计行按 `ON DELETE SET NULL` 保留（审计 append-only，未删审计）。
- 仅保留本报告文件 `docs/design/_tickets-qa-report.md`；未修改源码。

---

## 10. 复现附录（关键命令）

```bash
# 后端
cargo check -p qe-api
cargo test -p qe-domain
cargo test -p qe-api --lib
TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test \
TEST_REDIS_URL=redis://127.0.0.1:6379 \
cargo test -p qe-api --test tickets -- --ignored --test-threads=1
cargo test -p qe-api --no-run
cargo fmt --check -p qe-api -p qe-domain -p qe-db

# 前端
cd frontend
./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json
./node_modules/.bin/vitest run
./node_modules/.bin/vite build
./node_modules/.bin/oxlint src
E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 \
  ./node_modules/.bin/playwright test e2e/tickets.spec.ts --project=chromium --reporter=list
./node_modules/.bin/oxfmt --check src   # ❌ src/api/index.ts（问题 #9）

# 生成物一致性
curl -s http://127.0.0.1:8081/api-docs/openapi.json | jq -S . > /tmp/live.json
jq -S . frontend/docs/openapi.json > /tmp/file.json && diff /tmp/live.json /tmp/file.json   # 一致
python3 frontend/docs/openapi-types.py && ./node_modules/.bin/oxfmt frontend/src/api/schema.ts  # 重生成后一致
```

> 说明：开放问题 #2（occurrence 双计）与 #1（PATCH 无事件）建议发布前修复并补测试；其余 P2/P3 可排期。若产品确认「转派/改优先级」属于 P0 验收（#5），需在发布前补 UI。

---

## 11. 复验结果（第二轮，2026-11-01 20:00–20:05）

开发已按首轮报告修复并回归（含 migration `0019_ticket_event_updated.sql`；运行栈 8080/8081 + 5175 已用新二进制重启，`_sqlx_migrations.version=19 success=t`，`qestudio_test` 同样为 19）。本轮做**定向复验**（未全量重跑、未改源码、未重启服务）：

### 11.1 最终 verdict

**PASS** —— 首轮全部 P1 与绝大多数 P2/P3 已关闭并经实测确认；无 release blocker。剩余未关闭项均为**已确认的产品取舍 / 刻意行为 / 在途改动**（见 §11.5），不阻塞发布。

### 11.2 修复项逐条复验（实测证据）

| 原问题 | 修复验证 | 结果 |
|---|---|---|
| #1 PATCH 不写时间线 | `PATCH {title}` → 详情 events = `[created, updated{changed_fields:["title"]}]`；`PATCH {description}` → `updated{["description"]}`；`PATCH {priority+category}` → `updated{["category"]} + priority_changed{priority_to:"high"}`；migration 0019 在 dev/test 库均 applied；前端时间线 `updated` 映射为「更新内容」 | ✅ 关闭 |
| #2 occurrence 同源双计 | `bump_occurrence_tx` 增加 `NOT EXISTS (ticket_id, source_type, source_id)`（含 created 事件）后再插 occurred。双副本 live 实测：新 serial 连续异常 → 两个副本各自 tick 后自动单 `occurrence_count=1`、时间线仅 `created`（无 occurred）；schedule_delivery 自动单同样 =1。集成测试 `ticket_dedupe_bumps_occurrence_once_per_source` 通过 | ✅ 关闭 |
| #3 审计 from_status | live：claim（from 空→in_progress）后 transition resolve，审计 `detail.from_status=in_progress, to_status=resolved`（旧状态正确） | ✅ 关闭 |
| #4 schedule_delivery source_id | live 自动单 `source_id=8730876d-edf3-…`，与 fixture `schedule_events.id` **完全相同**（event_id，非 delivery_id）；delivery_id 仍保留 metadata | ✅ 关闭 |
| #5 前端无转派/改优先级入口 | 详情页新增「工单设置」弹窗（转派处理人/优先级/分类，`ticket:manage`）；Playwright 实测打开弹窗→改优先级 urgent→保存→toast「工单设置已更新」，DB `priority=urgent` + 新增 `priority_changed` 事件 | ✅ 关闭 |
| #6 transition 他人已分配不 409 | live：open 工单转派给 admin 后，operator 调 `transition{in_progress}` → **409**「工单已被他人认领」 | ✅ 关闭 |
| #7 assign 空 body 静默释放 | live：`POST /assign {}` → **400**「assignee_id 必填…」；`{"assignee_id":null}` → 200 且释放；传用户 id → 200 转派 | ✅ 关闭 |
| #8 PATCH 审计 action | live 审计 action=`ticket.update`、`detail.action=update_content`（3 次 PATCH 各 1 行）；契约文档同步补充 `ticket.update` | ✅ 关闭 |
| #9 oxfmt --check 失败 | `./node_modules/.bin/oxfmt --check src` → **All matched files use the correct format**（175 files） | ✅ 关闭 |
| #10 keyword ILIKE 通配符 | `keyword=%` live 实测 total 从「全部 8」降为 **1**，且该行是唯一真实含字面 `%` 的工单（`T-…-0007 磁盘使用率 93%`）；8080 副本结果一致；`escape_like` 转义 `\ % _` | ✅ 关闭 |
| #11 worker 日志/内存 | `seen_deliveries` 改为每轮用本轮候选 id 集合重建（`tracker.seen_deliveries = current_ids`，不再只增不减）；日志拆分为「自动工单已创建」与「自动工单重复发生已累加」，不再把 bump 当创建 | ✅ 关闭 |
| #12 OpenAPI 描述/必填 | live spec：`AssignTicketRequest.required=["assignee_id"]`、requestBody required=true；`mine` 描述改为「true = 只看 assignee_id = 当前用户；是否只看未结束由 active 参数控制」 | ✅ 关闭 |

### 11.3 关键命令复跑结果（全部通过）

| 命令 | 复验结果 |
|---|---|
| `cargo check -p qe-api` | ✅ Finished dev profile（0.69s） |
| `cargo test -p qe-domain` | ✅ **87 passed; 0 failed** |
| `cargo test -p qe-api --lib` | ✅ **61 passed; 0 failed**（ticket_worker/openapi 断言含新行为） |
| `cargo test -p qe-api --test tickets -- --ignored --test-threads=1` | ✅ **3 passed; 0 failed**（11.75s） |
| `vue-tsc --noEmit -p tsconfig.app.json` | ✅ exit 0 |
| `vitest run` | ✅ 7 files / **92 tests passed**（ticketMeta 17、ticketStats 18、TicketComponents 13、其余既有 39，无回归） |
| `vite build` | ✅ 成功 1.99s |
| `oxlint src` | ✅ 0 warnings / 0 errors |
| `oxfmt --check src` | ✅ 全部文件格式正确（首轮唯一失败项已修复） |
| Playwright e2e（`E2E_BASE_URL=http://127.0.0.1:5175 E2E_HEADLESS=1 --project=chromium`） | ✅ **3 passed**（10.9s） |
| 生成物一致性 | ✅ live `/api-docs/openapi.json` 与 `frontend/docs/openapi.json` canonical 一致；`TicketEventKind` 枚举含 `updated`；`schema.ts` 与重生成结果一致（e2e/vue-tsc 通过） |

### 11.4 契约与文档同步

- `docs/design/ticket-implementation-contract.md` 已同步修订（事件 kind 增加 `updated`、审计 action 增加 `ticket.update`、PATCH 行说明、验收命令/计数表更新）。即“冻结契约”已由负责人正式 re-baseline；建议在契约头部补一行 **修订版本/日期**（如 v1.1 / 2026-11-01）以便后续追溯（P3 文档建议，不阻塞）。
- 前端时间线文案、工单设置弹窗、`permissionGroups`/`permissions` 均随修复同步；`README.md`、`frontend/docs/CONVENTIONS.md` 已包含工单入口说明。

### 11.5 仍未关闭项及理由（均不阻塞发布）

| 原问题 | 状态 | 理由/后续建议 |
|---|---|---|
| #13 详情页 resolved/closed 隐藏评论框 | **保留（产品取舍）** | 后端允许评论；前端 `canComment` 排除终态，属有意交互设计。若产品希望「关闭后仍可留言」，去掉 `isTerminalStatus` 限制即可（改动 1 行 + 1 个测试）。 |
| #14 工作区混入 schedule-board 在途功能 | **保留（既有在途改动）** | 非工单缺陷；但 `openapi.rs`/`routes.ts`/`api/index.ts`/`openapi.json` 仍与排课工作台耦合，**发布/合并时二者需一起提交或拆分 commit**（发布流程注意项）。前端临时脚本建议提交前清理。 |
| #15 bootstrap 每次启动 bump 权限代际 | **保留（刻意行为）** | 为规避 5 分钟权限缓存导致新权限 403；建议发布说明写明「滚动发布会使各副本权限缓存失效一次」。 |
| P3 其他观察（comment 框、日志细节等） | 低风险 | 见 §5/#10-#15 说明；无发布阻塞。 |

### 11.6 本轮复验的局限

- 仍为**定向复验**：未全量重跑整个 workspace；未跑 Firefox/WebKit（仅 chromium）；未做 >2 副本并发压测（用 8080+8081 双副本实测同源 bump 不双计）；未复测与工单无关的排课工作台功能。
- 复验创建的临时工单/用户/worker fixture 已全部清理（dev 库恢复为原有 5 张工单）；审计按 append-only 保留。

