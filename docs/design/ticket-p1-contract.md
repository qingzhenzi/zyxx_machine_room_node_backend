# 工单 P1 实现契约（v2 / Wave 1）

> 状态：**已实现（Wave 1，2026-11-02）**。在 P0（`docs/design/ticket-implementation-contract.md`）基础上增量实现。
> 实现位置：`migrations/0021_ticket_rbac.sql`、`0022_ticket_rules_reminder.sql`、`crates/{domain,db,api}` 对应模块、`frontend/src/views/TicketRulesView.vue` + `features/tickets/*`（rules/SSE/export）。
> 验证见文末 §10；真实截图 `docs/design/ticket-implementation-rules.png`、`ticket-implementation-rule-modal.png`。
> Wave 1 范围：**独立细粒度 RBAC（含 ticket-only 角色）** + 规则配置化 + 三个新信号源 + SLA 超时提醒/邮件 + 工单 SSE + CSV 导出。
> Wave 2/3（后续轮次）：附件、按区域数据范围、根因分类、报修入口、巡检计划。不得破坏 P0 契约与既有测试。

## 1. 权限模型 v2（独立、可单独授权）

保留 P0 的 `ticket:read / create / handle / manage`（向后兼容），新增 7 个独立权限点：

| 权限 | 允许的动作 |
| --- | --- |
| `ticket:claim` | 认领、开始处理（`POST /claim`、`transition` 到 `in_progress`） |
| `ticket:comment` | 评论（`POST /comments`） |
| `ticket:transition` | 解决/关闭/重开/取消（`transition` 的其余目标状态；`in_progress` 见 claim） |
| `ticket:assign` | 转派/释放（`POST /assign`） |
| `ticket:update` | 改标题/描述/优先级/分类（`PATCH`） |
| `ticket:rule:manage` | 自动建单规则配置（`/admin/ticket-rules` 全部写操作 + 读取） |
| `ticket:export` | 导出 CSV（`/tickets/export`） |

**授权判定（OR 兼容）**：

| 接口 | 通过条件（任一） |
| --- | --- |
| list / summary / stats / get / stream | `ticket:read` |
| create | `ticket:create` |
| claim | `ticket:claim` 或 `ticket:handle` |
| comments | `ticket:comment` 或 `ticket:handle` |
| transition → in_progress | `ticket:claim` 或 `ticket:handle` |
| transition → 其他状态 | `ticket:transition` 或 `ticket:handle` |
| assign | `ticket:assign` 或 `ticket:manage` |
| PATCH | `ticket:update` 或 `ticket:manage` |
| assignees | `ticket:assign` 或 `ticket:manage` 或 `ticket:handle` |
| ticket-rules（读/写） | `ticket:rule:manage` 或 `ticket:manage` |
| export | `ticket:export` 或 `ticket:read` |

实现：handler 用 `authorization::require_any_permission(&state, id, &[新权限, 旧权限]).await?`；**不允许只有 `ticket:read` 的人执行任何写操作**。

**迁移 `0021_ticket_rbac.sql`**：
- 插入 7 个新权限（name/description 中文）。
- admin 全量授权（bootstrap 启动也会补）。
- `operator`（值班运维）：删除 `ticket:handle`，补 `ticket:claim/comment/transition`；保留 read/create 与 device/area/room read。
- **新增 `ticket_operator`（工单处理员，`is_system=FALSE`）**：只给 `ticket:read / create / claim / comment / transition`，**不捆绑任何 device/area/room 权限**（证明工单权限可独立授权；工单详情上下文由 ticket 接口自带，不依赖 device:read）。
- viewer/editor 保持 `ticket:read`。

`crates/domain/src/rbac.rs`：新增 7 常量 + ALL（25→32）+ 测试断言；新增 ticket 权限测试。

## 2. 自动建单规则配置化

**迁移 `0022_ticket_rules_reminder.sql`**：

```sql
CREATE TABLE ticket_rules (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    signal_type TEXT NOT NULL CHECK (signal_type IN (
        'machine_health','device_offline','schedule_delivery',
        'presence_delivery','transfer','image_verify')),
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    priority    TEXT NOT NULL DEFAULT 'normal'
        CHECK (priority IN ('urgent','high','normal','low')),
    scope_type  TEXT NOT NULL DEFAULT 'global'
        CHECK (scope_type IN ('global','area','room')),
    scope_id    UUID,                 -- scope_type=area/room 时必填（应用层校验）
    thresholds  JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER ticket_rules_set_updated_at ...;

ALTER TABLE tickets ADD COLUMN last_reminded_at TIMESTAMPTZ;
-- ticket_events.kind 增加 'reminded'（DROP/ADD ticket_events_kind_check）
```

种子（全部 enabled、global、priority=normal；machine_health critical 仍由 worker 映射为 high）：
`机器健康连续异常 {consecutive:2}`、`设备离线 {offline_minutes:10}`、`课表投递失败 {lookback_hours:24}`、`在线事件投递失败 {lookback_hours:24}`、`存储分发失败 {lookback_hours:24}`、`镜像验证失败 {lookback_hours:24}`。

**API（`/api/v1/admin`）**：

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/ticket-rules` | `ticket:rule:manage` 或 `ticket:manage` | 全量列表（按 signal_type, name 排序） |
| POST | `/ticket-rules` | 同上 | `{name,signal_type,enabled,priority,scope_type,scope_id?,thresholds}` |
| PATCH | `/ticket-rules/{id}` | 同上 | 部分更新（可空字段用 `deserialize_nullable`） |
| DELETE | `/ticket-rules/{id}` | 同上 | 删除（审计） |

- 校验：`signal_type` 合法；`thresholds` 必须是对象且键值范围合法（consecutive 1–10、offline_minutes 1–1440、lookback_hours 1–168）；`scope_type=area/room` 时 `scope_id` 必填且存在。
- 审计：`ticket_rule.create / update / delete`，`resource_type="ticket_rule"`。
- worker 每 tick 读取 enabled 规则；按规则 `thresholds`/`priority`/`scope` 生成候选；规则被禁用/删除则该信号不再自动建单。规则变更多副本下最多一个 tick 生效差异，可接受。

**前端**：新增 `/tickets/rules`（name `ticket-rules`，activeName `tickets`，权限 `ticketRuleManage` 或 `ticketManage`），侧边栏「运维」组第三项「自动规则」；页面表格（名称/信号/范围/阈值/优先级/启用/更新时间）+ 新建/编辑 `AppModal` + 删除 `ConfirmDialog`；阈值按 signal_type 渲染对应输入。

## 3. 新信号源（worker）

| signal_type | 数据源 | 聚合/去重键 | category | source_type/source_id | 默认优先级 |
| --- | --- | --- | --- | --- | --- |
| `presence_delivery` | `device_presence_deliveries.status='failed'` 且 `updated_at` 在 lookback 内 | `presence_delivery:{target_url}:{本地日期}` | `delivery_failed` | `presence_delivery` / delivery id | normal |
| `transfer` | `transfers.status IN ('failed','expired')` 且 `updated_at` 在 lookback 内 | `transfer:{transfer_id}` | `distribution_failed` | `transfer` / transfer id | normal |
| `image_verify` | 每个 artifact 最新一条 `image_verifications` 为 `ok=false`（且无更新的 ok=true） | `image_verify:{artifact_id}` | `distribution_failed` | `image_verify` / verification id | normal |

- 「本地日期」按房间所属区域时区（复用现有 `local_date_of` 逻辑）。
- 去重/occurrence/occurred 语义与 P0 完全一致（同源不双计）。
- 无房间映射时仍建单（room 为空），metadata 保存原始错误/摘要。

## 4. SLA 超时提醒 + 邮件

- 新配置：`TICKET_EMAIL_ENABLED`（默认 true；无 SMTP 时走现有 log driver）、`TICKET_OVERDUE_REMIND_HOURS`（默认 24，1–168）。
- `EmailService` 新增：
  - `send_ticket_assigned(email, code, title, link)`：转派成功后发给新处理人（有 email 时，best-effort，不阻塞响应）。
  - `send_ticket_overdue(email, code, title, link, due_at)`：超时提醒。
- worker 每 tick 扫超时：`due_at < now() AND status IN ('open','in_progress') AND (last_reminded_at IS NULL OR last_reminded_at < now() - remind_hours)`；对 assignee（无则 reporter，都没有则跳过）发邮件、写 `reminded` 事件（payload: due_at/assignee_id）、更新 `last_reminded_at`；单条失败只 warn。
- 前端详情时间线显示 `reminded`（「系统：超时提醒已发送」），不改现有超时角标逻辑。

## 5. 工单 SSE

- `GET /admin/tickets/stream?after_event_id=`（`ticket:read`）：先补发 `id > after_event_id` 的最多 200 条 `ticket_events`，之后每 2s 查增量，15s 心跳；帧 `event: ticket.event`，data 为 TicketEvent JSON（含 ticket_id/code/kind/status? 若需 code 则 JOIN tickets 返回 `ticket_code`）。
- 多副本安全：以查库为准，不依赖内存广播。
- 前端 `useTicketStream.ts`：fetch + ReadableStream（复用 `useMachineStream` 模式），断线 3s 重连，收到事件后 `useDebounceFn(400ms)` 刷新列表/详情；列表页右上角显示「实时」状态点。

## 6. CSV 导出

- `GET /admin/tickets/export`：接受与 GET `/tickets` 相同筛选参数（不含分页，最多 5000 行），返回 `text/csv; charset=utf-8`，UTF-8 BOM，表头中文：`编号,标题,状态,优先级,分类,来源,区域,房间,设备槽位,序列号,上报人,处理人,创建时间,解决时间,关闭时间,解决说明`。
- 权限：`ticket:export` 或 `ticket:read`。
- 前端列表页「导出 CSV」按钮：用 `fetch` 带 Authorization 拉 blob 下载，文件名 `tickets-YYYYMMDD-HHmm.csv`；失败 toast。

## 7. 文件归属（Wave 1）

| 负责人 | 文件 |
| --- | --- |
| backend-dev | `migrations/0021_ticket_rbac.sql`、`0022_ticket_rules_reminder.sql`、`crates/domain/src/{rbac,ticket}.rs`、`crates/db/src/{ticket_rules.rs,lib.rs,tickets.rs}`、`crates/api/src/api/admin/{ticket_rules.rs,tickets.rs,mod.rs}`、`crates/api/src/{ticket_worker.rs,email.rs,state.rs,config.rs,api/openapi.rs}`、`crates/api/tests/tickets.rs`、`.env.example` |
| frontend-pages-dev | `frontend/src/views/TicketRulesView.vue`、`frontend/src/features/tickets/{TicketRuleModal.vue,ticketRuleMeta.ts,useTicketStream.ts,__tests__/*}`、`TicketDetailView.vue`（权限门控 + reminded 文案）、`TicketsView.vue`（导出/实时）、`TicketTimeline.vue` |
| Lead | `frontend/src/utils/permissions.ts`、`router/routes.ts`、`AppSidebar.vue`、`features/roles/permissionGroups.ts`、`api/{index.ts,types.ts,schema.ts}`、`frontend/docs/openapi.json`、文档 |
| QA | 独立复验（只写 QA 报告） |

## 8. 验收

```bash
cargo fmt --check -p qe-api -p qe-domain -p qe-db
cargo check -p qe-api && cargo test -p qe-domain && cargo test -p qe-db && cargo test -p qe-api --lib
TEST_DATABASE_URL=postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test TEST_REDIS_URL=redis://127.0.0.1:6379 \
  cargo test -p qe-api --test tickets -- --ignored --test-threads=1
cargo run -q -p qe-api --example dump_openapi > frontend/docs/openapi.json
cd frontend && python3 docs/openapi-types.py && ./node_modules/.bin/oxfmt src/api/schema.ts
./node_modules/.bin/vue-tsc --build && ./node_modules/.bin/vitest run && ./node_modules/.bin/vite build
./node_modules/.bin/oxfmt --check src && ./node_modules/.bin/oxlint src
E2E_BASE_URL=http://127.0.0.1:5174 E2E_HEADLESS=1 ./node_modules/.bin/playwright test e2e/tickets.spec.ts --project=chromium
```

集成测试至少新增：独立权限矩阵（只有 comment 不能 claim；只有 read 不能写；ticket_operator 无 device 权限也能处理工单）、规则 CRUD/禁用后不再建单、三个新信号各建单一次、超时提醒写 reminded 且 last_reminded_at 更新、SSE 补发/增量、CSV 内容与筛选。

## 9. 前端生成类型与响应形状（必须一致）

`GET /admin/ticket-rules` → `TicketRuleList`；`POST /admin/ticket-rules` / `PATCH /admin/ticket-rules/{id}` → `TicketRule`；`DELETE` → `204 No Content`。

```jsonc
// TicketRule
{ "id": "uuid", "name": "机器健康连续异常", "signal_type": "machine_health",
  "enabled": true, "priority": "normal", "scope_type": "global", "scope_id": null,
  "thresholds": { "consecutive": 2 }, "created_at": "…", "updated_at": "…" }

// TicketRuleList
{ "items": [ /* TicketRule */ ], "total": 6 }

// CreateTicketRuleRequest
{ "name": "…", "signal_type": "machine_health", "enabled": true, "priority": "normal",
  "scope_type": "global", "scope_id": null, "thresholds": { "consecutive": 2 } }

// UpdateTicketRuleRequest（signal_type 不可改；其余可选）
{ "name": "…", "enabled": false, "priority": "high",
  "scope_type": "room", "scope_id": "uuid", "thresholds": { "offline_minutes": 15 } }

// TicketStreamEvent（SSE `event: ticket.event` 的 data）
{ "id": 123, "ticket_id": "uuid", "ticket_code": "T-20261101-0002",
  "kind": "status_changed", "status": "in_progress", "created_at": "…" }
```

约束：`thresholds` 为对象，键按 `signal_type` 校验（machine_health: consecutive 1–10；device_offline: offline_minutes 1–1440；其余: lookback_hours 1–168）；未知键 400。`scope_type=area/room` 时 `scope_id` 必填且必须存在。Update 请求中 `thresholds` 为整对象替换（不做深合并）。


## 10. 落地验证记录（2026-11-02，全部通过）

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --check` / `cargo check -p qe-api` | ✅ |
| `cargo test -p qe-domain` | ✅ 95 passed（含 TicketRule/阈值校验） |
| `cargo test -p qe-db` | ✅ 20 passed |
| `cargo test -p qe-api --lib` | ✅ 66 passed（含 worker 规则/SSE/OpenAPI 断言） |
| 工单集成（真库） | ✅ 4 passed（含 ticket-only 角色、规则 CRUD、CSV、P0 回归） |
| 前端 `vue-tsc --build` | ✅ 0 errors |
| 前端 `vitest run` | ✅ 97 passed（含 P1 rule-meta / SSE 助手单测） |
| 前端 `vite build` / `oxfmt --check` / `oxlint` | ✅ / ✅ / 0 warnings |
| Playwright e2e | ✅ 4 passed（P0 三条 + 自动规则页） |
| 迁移 | dev/test 库 `_sqlx_migrations` 0021/0022 success；6 条规则种子；11 个 ticket 权限 |
| 实测 | 规则 CRUD/400 阈值校验/204 删除；ticket_operator 无 device 权限可处理工单、PATCH/RULES 403；`/tickets/export` BOM+中文表头；SSE `ticket.event` 帧；admin 11 个 ticket 权限 |


## 11. Wave 2（2026-11-02）：根因 / 数据范围 / 附件

> 状态：**已实现**。迁移 `0023_ticket_root_cause.sql`、`0024_ticket_scope.sql`、`0025_ticket_attachment.sql`。

- **根因分类**：`tickets.root_cause` ∈ hardware/network/software/config/usage/other（可空）；`transition` 的 `root_cause` 在 resolve/close 时写入；详情展示、OpenAPI/前端类型同步。
- **数据范围**：权限 `ticket:scope:manage`（配置）/`ticket:scope:all`（不受限）；表 `ticket_scopes(user_id, area_id)`；`GET/PUT /admin/ticket-scopes[/{user_id}]`；无 `scope:all` 且配置了区域的用户只能看到/操作本区域工单（在管列表/统计/导出/详情/流转生效；未配置 = 不限制，向后兼容）。
- **附件**：表 `ticket_attachments`；权限 `ticket:attachment`；两种上传方式：
  - **代理上传（默认，推荐）**：`POST /tickets/{id}/attachments/upload?filename=&content_type=`，请求体为文件原始字节，后端计算 sha256、写对象存储并登记——同源调用，**不依赖对象存储 CORS**（修复 QA Wave2 blocker；需 `MAX_BODY_BYTES≥20MiB+余量`，dev `.env` 已设 25MiB）；
  - 预签名直传（可选，供其他客户端）：POST presign → PUT 对象存储 → POST register（对象存储需配 CORS）。
  - GET 返回下载预签名；DELETE 上传者或 `ticket:manage`；限制 20MiB、sha256、文件名/类型白名单；storage 未配置时 503。
- **前端**：
  - 详情页「根因分类」选择、附件上传/下载/删除；
  - `/tickets/scopes`「可见范围」页（用户 + 区域多选，`permissionAny: [scope:manage, manage]`）；
  - `?new=1&room_id=&serial=&device_slot=&title=` 预填建单 + 房间/设备/机器详情页「报修/建单」入口。
- **验证（2026-11-02）**：`cargo fmt/check` ✅；domain 99 / db 20 / api-lib 66 / tickets 集成 6 ✅；`vue-tsc` 0 错误；vitest 101 ✅；vite build / oxfmt / oxlint ✅；Playwright e2e 6/6（含报修预填与可见范围页）✅；实测 presign→PUT→register→list(download_url) 全链路、root_cause 落库、scopes 列表/过滤、manage-only 路由门控；OpenAPI 101 paths。


## 12. Wave2 QA 修复（2026-11-02）

- **附件上传 CORS blocker**：新增后端代理上传 `POST /admin/tickets/{id}/attachments/upload`（原始字节 + 服务端 sha256 + `put_bytes` + 登记），前端附件改用该接口，浏览器上传实测通过（无对象存储 CORS 依赖）；预签名直传保留给其他客户端（需 bucket CORS）。
- **RBAC 多段权限码 P2**：`normalize_permission_code` 允许 2–4 段（`ticket:rule:manage` / `ticket:scope:all` 等），仍拒绝空段/非法字符/超过 4 段；实测 `POST /admin/roles` 授予多段权限 201，自定义角色可获授 Wave1/Wave2 新权限。
- **环境**：`.env` / `.env.example` `MAX_BODY_BYTES=26214400`（25MiB，供 20MiB 附件代理上传）。


## 13. Wave 3（2026-11-02）：巡检计划

> 状态：**已实现**。迁移 `0026_ticket_inspection.sql`。

- **权限**：`ticket:inspection:manage`（`ticket:manage` 兼容）；admin 全量，operator/ticket_operator 不默认授予；`rbac::permissions` 已补常量（ALL 35→36）。
- **表**：`inspection_plans`（名称/范围 global·area·room/间隔 1–168h/启停/last_run_at/next_run_at）、`inspection_runs`（running·ok·issues·failed、checked/issue_machines、ticket_id、summary、error）。
- **Worker**：每 tick `claim_due_plans`（`FOR UPDATE SKIP LOCKED` + 前移 next_run_at，多副本安全）；查询 scope 内机器最新快照，异常机器 >0 → 建一张 `category=machine_health`、`source_type=inspection`、`source_id=run_id`、`dedupe_key=inspection:{plan}:{run}` 的工单（critical 时 priority=high），run 关联 ticket；无异常 → ok；执行失败 → failed，不阻塞其它规则。**手动 run 也会把 next_run_at 前移一个周期**，避免 next_run_at=NULL 时 worker 立刻重复执行。
- **API**：GET/POST `/inspection-plans`、PATCH/DELETE `/{id}`、POST `/{id}/run`、GET `/inspection-runs?plan_id=&limit=`；权限 + 审计 `inspection_plan.create/update/delete/run`。
- **前端**：`/tickets/inspections` 计划表格（范围/间隔/启停/最近运行/下次运行）、新建编辑弹窗、立即运行、删除、运行历史面板。
- **其它修复**：Axum `DefaultBodyLimit` 与 `RequestBodyLimitLayer` 同步 `MAX_BODY_BYTES`，21MiB 上传正确返回 400（而非连接被断）。
- **验证（2026-11-02）**：`cargo fmt/check` ✅；domain 100 / db 20 / api-lib 66 / tickets 集成 7 ✅；OpenAPI 106 paths；前端 vue-tsc 0 错误、vitest 112、vite build/oxfmt/oxlint ✅；Playwright e2e 7/7（含 rules/scopes/inspections/报修预填）✅；live：迁移 0026 应用、admin 获 inspection 权限、创建计划→手动运行→issues run + 巡检工单、next_run_at 前移、21MiB→400、1MiB→201。
- **QA**：`docs/design/_tickets-qa-report-p1-wave3.md`（PASS-with-issues，P3 不阻塞）；Wave1/2 QA 报告分别为 `_tickets-qa-report.md`、`_tickets-qa-report-p1-wave2.md`。


## 14. P3 收尾修复（2026-11-03）

- **附件下载**：新增 `GET /admin/tickets/{id}/attachments/{attachment_id}/download`（`ticket:read` + 数据范围），后端代理返回 `Content-Disposition: attachment`；前端改为 blob 下载，不再用 `window.open` 预览，也不依赖对象存储 CORS。
- **附件删除清理对象**：`StorageService::delete_object`（预签名 DELETE，404 幂等）；删除附件元数据后 best-effort 删除 S3 对象（失败仅告警）。实测删除后原预签名 URL → 404。
- **register 校验对象存在**：配置对象存储时 `HEAD` 校验 `object_key`，不存在 → 400。
- **范围页面区域选项**：新增 `GET /admin/ticket-scopes/areas`（scope 管理权限即可，不要求 `area:read`）；`/tickets/scopes` 改用它，低权限管理员也能选区域。
- **低权限概览 403 噪声**：`stores/overview.ts` 的 `optional` 改为**懒执行**（此前 `request` 作为参数会先发出），并按 `area/room/device/schedule/image/audit/storage/announcement:read` 门控；`stores/shell.ts` 同样按权限跳过房间/设备/在线事件请求。实测 `ticket_operator` 账号首页 403 网络请求为 0。
- **新增回归测试**：CSV 导出 >5000 行截断（5001 行数据 → 5001 行含表头）、超时提醒双副本原子抢占（两并发仅一次成功）、范围区域选项权限（admin 200 / viewer 403）。
- **验证**：backend fmt/check；domain 100 / db 20 / storage 17 / api-lib 66 / 工单集成 **10**；frontend vue-tsc 0 / vitest 112 / build/oxfmt/oxlint；e2e 7/7；live：代理下载内容一致、删除后对象 404、范围区域 200（13 个区域）、低权限首页 0 个 403。
