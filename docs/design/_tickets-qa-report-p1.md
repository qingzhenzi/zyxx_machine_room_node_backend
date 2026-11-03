# 工单 P1 Wave 1 独立 QA 验证报告

> 验证人：qa-verifier（独立 QA）｜时间：2026-11-02｜对象：`room_node_backend` 工单 P1 Wave 1
> 依据：`docs/design/ticket-p1-contract.md`（v2/Wave1）+ `docs/design/ticket-implementation-contract.md`（P0 基线）
> 方式：只读代码审查 + 契约验收命令 + 真实 PG/Redis 集成测试 + 运行栈（API `127.0.0.1:8080`、Vite `127.0.0.1:5174`）的 API/DB/UI/Playwright 实测。
> 只写本报告文件，未改源码；临时数据已清理（§10）。验证期间（13:26:20）后端被**其他并行任务**重启为新二进制一次，导致一次 worker 基线重置（影响项已用"基线后新 fixture"复测，见 §5）。

---

## 1. 结论（verdict）

**PASS-with-issues** —— **未发现 release blocker**。P1 Wave 1 的核心交付（11 权限点 + ticket_operator 独立角色 + OR 兼容、规则 CRUD/校验/审计/禁用与删除后停止建单、范围过滤与优先级生效、三个新信号的 worker 真实建单与同源去重、SLA 超时提醒/`reminded`/邮件、SSE、CSV、前端规则页/细粒度门控/导出/实时/reminded 文案）均实测通过；P0 回归通过。剩余问题为 **2 个 P2（验收命令格式失败、前端规则页路由门控与契约 OR 语义不符）** 与若干 P3 改进项。

问题按严重度排序：

| # | 严重度 | 问题 | 位置 |
|---|--------|------|------|
| 1 | P2 | 前端 `/tickets/rules` 路由与侧边栏**只按 `ticket:rule:manage` 单权限**控制；合同要求 `rule:manage` **或** `ticket:manage`，导致仅有旧 `ticket:manage` 的管理员被重定向 `/forbidden`、侧边栏无入口（后端 GET rules 返回 200）。实测：manage-only 导航 `/tickets/rules` → `/forbidden?from=/tickets/rules`，navItem=0 | `frontend/src/router/routes.ts:148-155`、`frontend/src/router/guards.ts:45-49`、`frontend/src/components/layout/AppSidebar.vue:143-147` |
| 2 | P2 | `cargo fmt --check -p qe-api -p qe-domain -p qe-db` **失败**（契约 §8 验收命令）：`ticket_rules.rs` 2 处超长行、`tickets.rs` 2 处换行风格 | `crates/api/src/api/admin/ticket_rules.rs:87,94`；`crates/api/src/api/admin/tickets.rs:728,740` |
| 3 | P2 | `./node_modules/.bin/oxfmt --check src` **失败**（1 file）：`frontend/src/views/TicketDetailView.vue` 未格式化（验收命令） | `frontend/src/views/TicketDetailView.vue` |
| 4 | P3 | **CSV 公式注入**：`csv_field` 只处理逗号/引号/换行，未中和 `=`/`+`/`-`/`@`；实测标题 `=1+1` 原样导出（`title_fields=['=1+1']`），Excel 打开会被当公式执行 | `crates/api/src/api/admin/tickets.rs` `csv_field` |
| 5 | P3 | `reminded` 事件 payload 只写 `{"due_at":…}`，契约 §4 要求 `due_at/assignee_id`；实测事件 payload 缺 `assignee_id` | `crates/db/src/tickets.rs` `mark_reminded` |
| 6 | P3 | 超时提醒**多副本并发无原子占用**：`overdue_tickets` 普通 SELECT + `mark_reminded` 无条件 UPDATE/插入 → 两副本可能同一窗口重复发信、重复写 `reminded`。建议 `UPDATE tickets SET last_reminded_at=now() WHERE id=$1 AND (last_reminded_at IS NULL OR last_reminded_at < now()-window) RETURNING` 原子抢占后再发信/写事件 | `crates/db/src/tickets.rs:1652-1705`、`crates/api/src/ticket_worker.rs:289-323` |
| 7 | P3 | `image_verify` 规则的 scope 过滤恒为 `matches_scope(None,None)`：**room/area 范围的 image_verify 规则永远不命中**，但前端允许创建 → 静默无效。建议在规则弹窗禁用/提示，或禁止建 image_verify 的非 global 规则 | `crates/api/src/ticket_worker.rs:282` |
| 8 | P3 | 同一 `signal_type` **允许存在多条启用规则**（DB 无唯一约束）；worker 按 `(signal_type,name)` 顺序逐条执行，machine_health/offline 共享进程内 tracker，先到先得——更具体的 room/area 规则可能被排序更靠前的 global 规则遮蔽。建议按 scope rank 取最具体一条，或对 signal_type+scope 加唯一约束 | `crates/api/src/ticket_worker.rs:87-107`、`migrations/0022_ticket_rules_reminder.sql` |
| 9 | P3 | 详情页「工单设置」门控为 `canAny(assign,update,manage)`，但保存时 assign 与 PATCH 分别鉴权：assign-only 用户改优先级会 403、update-only 用户看到转派下拉（assignees 加载 403 静默），且错误文案仍写“需要 ticket:manage”。建议按权限禁用对应字段并更新文案 | `frontend/src/views/TicketDetailView.vue:104-110,277-300` |
| 10 | P3 | e2e 首轮因 Vite **冷启动编译**>30s 导致 3 个用例 `page.goto` 超时失败；预热后复跑 **4/4 通过**。属测试环境抖动，建议 e2e 前置预热或本地也开启 1 次重试 | `frontend/e2e/tickets.spec.ts`、playwright 配置 |
| 11 | P3 | 低权限角色（ticket_operator/viewer/manage-only）在控制台产生大量 403 与 `pageerror: ApiError 缺少权限：audit:read/storage:read/announcement:read`（shell 概览组件未按权限跳过）。非工单页专属、疑似既有行为，但 P1 新角色使其常态化；建议 shell store 按权限跳过请求 | `frontend/src/stores/*`、概览页组件 |
| 12 | P3 | CSV 导出 **≤5000 行上限**未在规模数据下验证（dev 仅 14 行）；SQL `LIMIT 5000` 已代码核对。另：导出错误提示只写“没有 ticket:export 权限”，实际 `ticket:read` 也可导出（契约 OR） | `crates/api/src/api/admin/tickets.rs:696-740`、`frontend/src/views/TicketsView.vue` |
| 13 | P3 | 规则 PATCH 中 `signal_type` 被 serde 静默忽略（满足“不可改”但无 400 反馈）；`reminded` 前端文案为「超时提醒已发送」（actor 列已显示“系统”，与契约“系统：超时提醒已发送”语义等价，可接受） | `crates/api/src/api/admin/ticket_rules.rs:55-70`、`frontend/src/features/tickets/TicketTimeline.vue:35-45` |

---

## 2. 环境与方法

- 后端 `127.0.0.1:8080`（migration `0021_ticket_rbac` + `0022_ticket_rules_reminder` 已应用，`_sqlx_migrations` 21/22 success=t；6 条种子规则 enabled/global/normal）。
- 前端 Vite `127.0.0.1:5174`；admin/Admin@12345。
- 验证中运行时后端被其他任务重启（新二进制 13:25:35，重启 13:26:20），新进程重新建立 worker 基线；受影响的一次 priority 用例已用“基线之后新建 fixture”复测通过（§5.3）。
- 未重启/未 kill 任何服务（后端重启非本次操作）、未改源码、未新增依赖、无外网请求。

---

## 3. 验收命令结果（逐条实测）

| # | 命令 | 结果 |
|---|------|------|
| 1 | `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | ❌ **FAIL**（4 处 diff：ticket_rules.rs:87,94；tickets.rs:728,740） |
| 2 | `cargo check -p qe-api` | ✅ Finished dev profile（2m50s 首次/缓存后 0.69s） |
| 3 | `cargo test -p qe-domain` | ✅ **95 passed; 0 failed**（含 P1 权限/规则阈值/scope 纯函数用例） |
| 4 | `cargo test -p qe-db` | ✅ **20 passed; 0 failed** |
| 5 | `cargo test -p qe-api --lib` | ✅ **66 passed; 0 failed**（含 worker 新信号规划/SSE/export 单测） |
| 6 | `cargo test -p qe-api --test tickets -- --ignored --test-threads=1`（`TEST_DATABASE_URL=…/qestudio_test TEST_REDIS_URL=…:6379`） | ✅ **4 passed; 0 failed**（10.81s；新增 `p1_rbac_rules_export_and_ticket_only_role`） |
| 7 | `./node_modules/.bin/vue-tsc --build` | ✅ exit 0 |
| 8 | `./node_modules/.bin/vitest run` | ✅ 8 files / **97 tests passed**（新增 `ticketRuleMeta.spec.ts` 5 个；P0 既有套件无回归） |
| 9 | `./node_modules/.bin/vite build` | ✅ 成功 1.79s |
| 10 | `./node_modules/.bin/oxfmt --check src` | ❌ **FAIL**：`src/views/TicketDetailView.vue`（190 files 中 1 个） |
| 11 | `./node_modules/.bin/oxlint src` | ✅ 0 warnings / 0 errors（189 files） |
| 12 | `E2E_BASE_URL=http://127.0.0.1:5174 E2E_HEADLESS=1 playwright test e2e/tickets.spec.ts --project=chromium` | ⚠️ 首轮 3 failed（Vite 冷启动 `page.goto('/login')` 30s 超时）；预热后复跑 ✅ **4 passed**（含新增“渲染自动规则页与种子规则”用例，12.7s） |
| 13 | 生成物一致性 | ✅ live `/api-docs/openapi.json` 与 `frontend/docs/openapi.json` canonical **完全一致**；live spec 含 13 条 ticket 路径（含 `/ticket-rules`、`/tickets/export`、`/tickets/stream`）与 `TicketRule/TicketRuleList/TicketStreamEvent` 等 schema；按脚本重生成 `schema.ts` 类型语义一致（临时目录缺少项目格式上下文，仅换行风格差异；仓库文件通过 oxfmt check） |
| 14 | 文档 | ✅ `README.md`（P0+P1 章节、权限/规则/API 表）、`frontend/docs/CONVENTIONS.md`（自动规则页归属）、`.env.example`（`TICKET_EMAIL_ENABLED`/`TICKET_OVERDUE_REMIND_HOURS`）均已更新且与实现一致 |

---

## 4. RBAC 矩阵实测（11 权限点 / OR 兼容 / 独立授权）

DB 实测权限/角色：
- 11 个 `ticket*` 权限全部存在；`ALL.len()=32`（domain 测试断言）。
- `ticket_operator` = `ticket:read/create/claim/comment/transition`（**无任何 device/area/room 权限**）。
- `operator` = `area/device/room:read + ticket:read/create/claim/comment/transition`（无 handle/manage）。
- `viewer/editor` = 仅 `ticket:read`（+既有只读权限）。

实测矩阵（临时用户，全部按契约预期，30+ 断言 PASS）：
- **ticket_operator**：list/summary/stats/detail 200；create 201；claim 200；comment 201；transition 200；**devices/rooms/areas 403**（无 device/area/room 权限，工单详情上下文仍可用）；PATCH 403；assign 403；rules 403；assignees 403（无 handle/assign/manage）。
- **operator（旧角色）**：claim/comment/transition 200（`ticket:claim/comment/transition` 生效）；PATCH/assign/rules 403。
- **viewer**：list 200；claim/comment/transition/PATCH/assign/rules/assignees 全部 403（“只有 read 不能写”成立）。
- **OR 兼容（只给旧权限）**：`ticket:handle`-only → claim/comment/transition/assignees 200，rules/PATCH/assign 403；`ticket:manage`-only → PATCH/assign/rules GET/POST/DELETE 全 200/201/204，claim/comment/transition 403。
- **细粒度单权限**：`ticket:comment`-only → comment 201，claim/transition/PATCH 403；`ticket:export`-only → export 200，list/detail/stream/rules 403。
- **assignee 目标能力**：转派给 viewer（无 claim/comment/transition/handle）→ **400**“目标处理人没有 ticket:handle 权限”；转派给 ticket_operator → 200。
- assignees 列表返回 admin + 具备处理能力的用户（不含 viewer）。

---

## 5. 规则配置化 + worker 真实 tick

### 5.1 CRUD / 校验 / 审计（admin）
- `GET /ticket-rules` → `{items,total}`，total=6，按 signal_type,name 排序；默认值正确（enabled=true、priority=normal、scope=global）。
- 校验（全部 400）：空名字、非法 signal_type、阈值下界(consecutive=0)、未知阈值键(machine_health 传 lookback_hours)、阈值非整数、缺 thresholds（非对象）、area 缺 scope_id、area/room scope_id 不存在；PATCH 阈值越界 400；PATCH 不存在 404；DELETE 204→重复 DELETE 404。
- 权限：viewer/operator/ticket_operator GET rules 403（契约）；manage-only GET 200 且可 CRUD。
- 审计：`ticket_rule.create/update/delete` 均落库（`resource_type="ticket_rule"`，实测 3 类动作）。
- PATCH scope 组合：room 无 scope_id → 400；改回 global + `scope_id:null` → 清空成功；`signal_type` 被忽略（不可改）。

### 5.2 禁用/删除规则后停止建单（真实 worker tick）
- **禁用**：PATCH machine_health 规则 enabled=false 后注入新 serial 连续 2 条 warning 快照 → 等待 100s **建单数=0** ✅；重新启用后下一个 tick 自动建单（normal，source=auto）✅。
- **删除**：DELETE transfer 规则（204，审计 `ticket_rule.delete`，rules total 6→5）后插入 `transfers.status='failed'` → 等 75s **建单数=0** ✅；重建规则后下一个 tick 建单（`distribution_failed|transfer`，occurrence=1）✅。

### 5.3 优先级 / 范围过滤（真实 worker tick）
- **priority 生效**：规则 priority=urgent 后（基线之后新建 serial fixture）→ 自动单 `priority=urgent` ✅（验证中后端重启导致首轮基线重置，已用新 fixture 复测通过）。
- **room 范围过滤**：临时房间 R1/R2 各绑 serial；创建 room-scoped machine_health 规则（R1，priority=high），同时暂时禁用 global 规则 → 只建 R1 的自动单（`priority=high`、`room_id=R1`），R2 建单数=0 ✅。

---

## 6. 三个新信号源（真实 worker tick，基线之后注入）

| 信号 | 实测 ticket 字段 | 结果 |
|---|---|---|
| `presence_delivery` | `category=delivery_failed`、`source_type=presence_delivery`、`source_id=delivery id(4)`、dedupe `presence_delivery:{target_url}:{本地日期}`、occurrence=1 | ✅ |
| `transfer` | `category=distribution_failed`、`source_type=transfer`、`source_id=transfer uuid`、dedupe `transfer:{id}`、occurrence=1 | ✅ |
| `image_verify` | `category=distribution_failed`、`source_type=image_verify`、`source_id=verification uuid`、dedupe `image_verify:{artifact_id}`、occurrence=1 | ✅ |
| `schedule_delivery`（P0 回归） | `source_id = event_id`（实测 `273cdfea-…` == schedule_events.id）、`category=delivery_failed`、occurrence=1 | ✅ |

- **同源不双计**：以上 4 条自动单在再等一个 tick 后均为 `occurrence_count=1` 且时间线仅 `created`（无 occurred 重复）✅。
- 无房间映射的 presence 事件可建单（room 为空）✅（presence 事件的 room/area 为 null 时仍建单）。

---

## 7. SLA 超时提醒 + 邮件

- 构造：临时用户（有 email、ticket_operator）+ urgent 工单，admin 转派，DB 将 `due_at` 改为 2h 前。
- 下一个 tick 实测：
  - `ticket_events.kind='reminded'` 写入（事件 id=203），`tickets.last_reminded_at=2026-11-02T05:37:22.88Z` 更新 ✅。
  - 邮件 best-effort：后端日志 `开发邮件发送器：工单已超时：T-…（未配置 SMTP…） email="qa_p1_rem_…@example.com"` ✅（log driver，收件人为 assignee）。
  - **窗口内不重复提醒**：再等 70s（>1 tick），`reminded` 事件数仍=**1**、`last_reminded_at` 不变 ✅。
- 契约偏差：reminded payload 仅 `{due_at}`，缺 `assignee_id`（见问题 #5）；多副本并发去重未验证（见 #6、§11）。

---

## 8. SSE（`GET /admin/tickets/stream?after_event_id=`）

- 首次补发（after_event_id=0）：收到 **58 帧** `event: ticket.event`（≤200），data JSON 含 `id/ticket_id/ticket_code/kind/status/actor_name/note/payload/created_at` ✅。
- 增量：以 `max(id)` 起流后创建评论，2–4s 内收到该评论帧（`kind:"commented"`，ticket_code/status 正确）✅；同时收到 **1 帧 `event: heartbeat`**（15s 心跳）✅。
- 权限：无 `ticket:read` 用户（export-only）→ **403** ✅；`after_event_id=-1` → **400** ✅；未来 id → 200 空流（正常）。
- 后端重启后复测通过（查库为准，多副本安全）✅。

## 9. CSV 导出

- `GET /admin/tickets/export`：200，`content-type: text/csv; charset=utf-8`，`content-disposition: attachment; filename="tickets-YYYYMMDD-HHmm.csv"`；UTF-8 BOM（首字节 `efbbbf`）✅。
- 表头与契约完全一致（16 列：编号,标题,状态,优先级,分类,来源,区域,房间,设备槽位,序列号,上报人,处理人,创建时间,解决时间,关闭时间,解决说明），全部数据行 16 列 ✅。
- 筛选：`status=cancelled` 导出仅 1 行且状态=已取消，与列表一致 ✅；`page/page_size` 被忽略（page=1 与 page=9 结果相同）✅。
- 权限：`ticket:export`-only 200；`ticket:read` 用户 200（契约 OR）；无 read/export 用户 403 ✅。
- 上限：代码 `LIMIT 5000` 已核对；未在 >5000 行规模实测（§11）。
- 安全：标题 `=1+1` 原样导出（公式注入风险，问题 #4）。

## 10. 前端（`/tickets/rules`、门控、导出、实时、reminded）

- **自动规则页**：admin `/tickets/rules` 标题/6 行种子规则/新建按钮正常，无 console error；侧边栏「运维」组第三项「自动规则」在 admin 可见；路由 `/tickets/rules` 在 `:id` 前、`activeName=tickets`。
- **细粒度门控**（Playwright 实测）：
  - ticket_operator：详情 action=3（开始处理/认领/取消）、评论框可见、工单设置隐藏、自动规则 nav 隐藏、导出按钮可见。
  - viewer：无 action 按钮、无评论框、无工单设置、导出按钮可见（read OR export）、自动规则 nav 隐藏。
  - manage-only（旧权限）：工单设置可见、评论框隐藏；**自动规则 nav 隐藏且直接访问被 `/forbidden` 拦截**（问题 #1）。
- **导出按钮**：admin 列表页点击触发下载 `tickets-20261102-1329.csv`，与后端筛选参数一致 ✅。
- **实时指示**：列表页显示「实时」状态（`useTicketStream` 状态点，实测 liveIndicator=1），收到事件后防抖刷新列表/KPI。
- **reminded 文案**：详情时间线显示「超时提醒已发送」（图标 bell），实测 1 条 ✅。
- 无 `v-html`；未新增前端依赖（`git diff package.json` 空）；403/404/空态/错误态存在（P0 已验证，P1 未发现回归）。
- 控制台噪声：低权限角色在 shell 概览触发 `audit:read/storage:read/announcement:read` 403 + unhandled rejection（问题 #11）。

---

## 11. 未验证项（明确标注）

1. **CSV ≤5000 行规模**：dev 仅 14 行，未构造 5001 行验证截断（代码 LIMIT 5000 已审查）。
2. **提醒多副本并发**：当前仅单实例运行，未压测双副本同时提醒（代码无原子 claim，风险见 #6）。
3. **真实 SMTP 发信**：仅验证 log driver（契约允许无 SMTP 走 log），未配 SMTP 端到端。
4. **area-scoped 规则 live**：room-scoped 已 live 验证；area 分支同一 `matches_scope` 代码，仅代码审查。
5. **规则变更多副本 tick 差异**：未双副本验证（契约允许“最多一个 tick 差异”）。
6. **SSE backlog 200 上限边界**：实测 58 帧，未构造 >200 事件。
7. **Firefox/WebKit e2e**：仅 chromium（任务指定）。
8. **全仓 `cargo test`**：按任务范围只跑指定 crate/套件（未跑其它 crate 集成测试）。
9. **真实删除规则后的 75s 等待**中可能有其它并行任务写入，但断言期间规则确实不在 enabled 集合（worker 日志无该 signal 建单）。

## 12. 数据清理

- 已删除本次 QA 创建的全部临时数据：11 张工单（P1-QA 手工单/e2e 单/自动单）、1 个 image artifact、4 个临时角色、7 个临时用户；worker 自动清理了 QA-P1 serial/房间/enrollment/快照/presence 事件、规则恢复 6 条种子。
- dev 库恢复：**5 张原演示工单 + 6 条种子规则 + 原有用户/角色**；审计按 append-only 保留（QA 用户删除后 actor_id 置空）。

## 13. 复现命令（关键）

```bash
cd /home/qingzhenzi/Projects/QEStudio/room_node_backend
cargo fmt --check -p qe-api -p qe-domain -p qe-db          # ❌ 4 处（问题 #2）
cargo check -p qe-api && cargo test -p qe-domain && cargo test -p qe-db && cargo test -p qe-api --lib
TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test \
TEST_REDIS_URL=redis://127.0.0.1:6379 \
  cargo test -p qe-api --test tickets -- --ignored --test-threads=1
cd frontend
./node_modules/.bin/vue-tsc --build && ./node_modules/.bin/vitest run && ./node_modules/.bin/vite build
./node_modules/.bin/oxfmt --check src                      # ❌ TicketDetailView.vue（问题 #3）
./node_modules/.bin/oxlint src
E2E_BASE_URL=http://127.0.0.1:5174 E2E_HEADLESS=1 \
  ./node_modules/.bin/playwright test e2e/tickets.spec.ts --project=chromium

# 规则：禁用/删除后 worker 不建单、范围/优先级（真实 tick，约 60s/轮）
# RBAC：见 §4 矩阵（11 权限 / OR 兼容 / ticket_operator 无 device 权限）
# SSE/CSV：见 §8/§9；提醒：见 §7
```

---

## 14. 复验结果（第二轮定向复验，2026-11-02 13:48–13:56）

开发已按本报告修复并自测；本轮只做**定向复验**（未全量重跑、未改源码）。运行时后端/前端仍为 8080/5174，DB 由 Lead 清理为 5 演示工单/6 种子规则，复验后再次清理为同状态。

### 14.1 最终 verdict

**PASS-with-issues**（维持首轮结论；首轮 **P2 全部关闭**，P3 剩余项均为**已确认的改进项/测试环境问题**，无 release blocker）。

### 14.2 修复项逐条复验

| 原问题 | 修复验证 | 结果 |
|---|---|---|
| P2-1 前端规则页路由/侧边栏 OR 门控 | `guards.ts` 支持 `meta.permissionAny`（`canAny`）；`routes.ts:153` 与 `AppSidebar.vue:148` 改为 `[ticket:rule:manage, ticket:manage]`。Playwright 实测 manage-only 用户（仅 `ticket:read + ticket:manage`）：侧边栏「自动规则」**可见=1**，`/tickets/rules` **可直接进入**（不再 /forbidden），6 行种子规则 + 新建按钮正常 | ✅ 关闭 |
| P2-2 `cargo fmt --check` | 复跑 **FMT_EXIT=0**（qe-api/qe-domain/qe-db） | ✅ 关闭 |
| P2-3 `oxfmt --check src` | 复跑 **All matched files use the correct format**（190 files） | ✅ 关闭 |
| P3-4 CSV 公式注入 | `csv_field` 对 `= + - @ \t \r` 前缀加单引号；实测建单标题 `=1+1` → 导出 CSV 标题字段为 `'=1+1`（python csv 解析确认） | ✅ 关闭 |
| P3-5 reminded payload 缺 assignee_id | `claim_reminder` 原子 UPDATE… RETURNING `assignee_id,due_at`，事件 payload 写 `{due_at, assignee_id}`。实测 reminded 事件 payload = `{"due_at":"…","assignee_id":"370364d4-…"}`，`last_reminded_at` 同步更新，email log 正常 | ✅ 关闭 |
| P3-6 提醒多副本无原子占用 | `mark_reminded` 改为 `claim_reminder(id, remind_hours)`（UPDATE … WHERE `last_reminded_at` 为空或超窗 RETURNING），worker 先抢占成功才发信/写事件（代码审查；单实例实测抢占后水位更新） | ✅ 关闭（双副本并发未压测，见 §14.4） |
| P3-7 image_verify 非 global 规则静默失效 | 创建 API 校验：image_verify + 非 global → **400**“镜像验证是全局信号，规则范围只能是 global”；前端规则弹窗 `scopeSupported` 对 image_verify 禁用范围选择（`:disabled="!scopeSupported"`） | ✅ 关闭 |
| P3-8 同 signal 多规则顺序遮蔽 | worker 新增选择逻辑：同 `signal_type` 只执行 scope rank 最高的一条（room > area > global）。实测：保留 global 种子规则(normal) + 新建同 signal 的 room-scoped 规则(high)，基线后新 serial → 自动单 **priority=high 且 room_id=该房间**（global 未遮蔽 room 规则） | ✅ 关闭 |
| P3-13 PATCH signal_type 静默忽略 | 显式传不同值 → **400**“规则信号类型不可修改”；传相同值 → 200（name 正常更新） | ✅ 关闭 |

### 14.3 复跑命令结果（全部通过，除 e2e 见 §14.4）

| 命令 | 复验结果 |
|---|---|
| `cargo fmt --check -p qe-api -p qe-domain -p qe-db` | ✅ exit 0 |
| `cargo check -p qe-api` | ✅ Finished（26.48s） |
| `cargo test -p qe-domain` | ✅ 95 passed |
| `cargo test -p qe-db` | ✅ 20 passed |
| `cargo test -p qe-api --lib` | ✅ 66 passed |
| `cargo test -p qe-api --test tickets -- --ignored --test-threads=1` | ✅ 4 passed（24.2s） |
| `vue-tsc --build` | ✅ exit 0 |
| `vitest run` | ✅ 8 files / 97 passed |
| `vite build` | ✅ 成功 |
| `oxfmt --check src` | ✅ 全部格式正确 |
| `oxlint src` | ✅ 0 warnings / 0 errors |
| `playwright e2e --project=chromium` | ⚠️ **本轮 3 次运行均有页面加载超时 flake**：run1 2 failed/2 passed（冷启动）、run2 3 passed/1 failed（reload 后筛选恢复超时）、run3 2 failed/2 passed（`page.goto('/login')` 30s 超时）。失败均为**页面加载/等待超时**，非断言/功能失败；同一套件在首轮验证中曾 **4/4 全过**。判断为长时间运行的 Vite dev server 冷/慢编译导致的环境抖动（P3 #10），建议 e2e 前重启 dev server 或用 `vite preview` 构建产物跑 e2e（产品逻辑无回归） |

### 14.4 仍未关闭项（均不阻塞）

- **P3-9 设置弹窗细粒度文案/字段**：未在本次修复范围（assign-only 用户改优先级仍会 403，文案仍提 ticket:manage）。
- **P3-10 e2e 冷启动/长时间 dev server flake**：本轮持续复现页面加载超时；建议改 preview build 跑 e2e 或加本地重试。
- **P3-11 低权限角色 shell 403 噪声**：仍存在（audit:read/storage:read/announcement:read 403 + unhandled rejection），非工单页专属。
- **P3-12 CSV ≤5000 行规模**：仍未在 >5000 行数据下验证（`LIMIT 5000` 代码核对）；导出错误提示文案仍只提 ticket:export（read 亦可导出）。
- **未验证**：提醒双副本并发（单实例 + 代码审查）、真实 SMTP（仅 log driver）、area-scoped 规则 live（room-scoped 已 live）、SSE >200 backlog 边界、Firefox/WebKit、全仓 cargo test。
- 数据：复验创建的临时用户/角色/工单/规则/房间/serial 已全部清理，dev 库恢复 **5 张演示工单 + 6 条种子规则**。

