# 工单系统前端调研：壳层接线、页面模式、API 与 P0 落地规格

> 范围：`frontend/`（Vue 3 + Vite + Tailwind 4 + Pinia）。只读调研，**未改动 `frontend/src/` 任何文件**。
> 目的：回答「工单要做几个页面、要不要单独 layout、历史分析怎么呈现」，并给出可直接排期的前端 P0 规格。
> 依据：`frontend/docs/CONVENTIONS.md`、`frontend/docs/api-contract.md`、`frontend/docs/openapi.json` 与源码；
> 后端契约以 `docs/design/_tickets-backend-inventory.md`（task-1）为准，本文只记录**影响前端交互的事实与决定**；
> 字段命名与接口以 **Lead 最终决策**为准（见下方注记）；正文中的 `kind`/`severity` 属早期命名。
> 现状事实：仓库内目前**没有任何 ticket/工单代码**（全仓 `grep -rni "ticket|工单"` 无命中），下列工单条目均为新建。
>
> **命名/接口最终口径（Lead 决策，以此为准）**：最终契约采用 `category`（`machine_health | device_offline | delivery_failed | distribution_failed | security | other`）与 `priority`（`urgent | high | normal | low`），**不采用 kind/severity**——本文正文中两者只是同一字段的早期命名，无需逐行改。API 以 `docs/design/ticket-system-analysis.md` §6 为准：`GET /admin/tickets`（支持 `mine`/`active`）、`GET /admin/tickets/summary`（顶栏角标/计数，替代正文的两次查询）、`POST /admin/tickets/{id}/claim`、`POST /admin/tickets/{id}/assign`、`POST /admin/tickets/{id}/transition`、`PATCH /admin/tickets/{id}`（仅内容/优先级/分类）、`POST /admin/tickets/{id}/comments`、`GET /admin/tickets/assignees`、`GET /admin/tickets/stats`（P0 就做，不降级）；正文 §4.2 的 open/in_progress 两次查询与 §5.5 的旧方法/降级说明均以此覆盖。

---

## 0. 结论速览（给 Lead）

1. **不需要新布局**：工单复用 `DefaultLayout`（证据 §1.1、§1.7），独立布局只在值班大屏 / 接单台 / 匿名报修表单三类场景才考虑，P0 均不做。
2. **三条路由**：`/tickets`（列表 + 新建弹窗）、`/tickets/insights`（历史分析）、`/tickets/:id`（独立详情路由，非抽屉，符合 `docs/CONVENTIONS.md:80`）。
3. **导航新增「运维」分组**，位于「资源」之后、「管理」之前：工单 / 工单分析。`AppSidebar.vue:23-160` 是数据驱动，不改分组模型。
4. **4 个权限点** `ticketRead/Create/Handle/Manage` → `utils/permissions.ts:6-28`，并补角色编辑页分组（`features/roles/permissionGroups.ts:10-34`）。
5. **通知中心最小扩展**：新增 `stores/ticketTodo.ts`（不改 `shell.ts`），菜单加「我的待办工单」分组与独立 seen 键；presence 的 `rn:presence:seen-id` 逻辑完全不动（`shell.ts:8-25`）。
6. **不引入新依赖**（`CONVENTIONS.md:85`）：趋势/分布图用 HTML + CSS 宽高百分比，复用 `ProgressBar.vue:26-40`、`UsageBar.vue:31-56` 写法；P0 前端约 7–8 人天（列表 2d、详情 2.5d、分析 1.5d、通知 0.5d、测试联调 1.5d）。
7. **需要 Lead 决策**：① 接口与权限码最终命名（task-1）；② `NotificationsMenu.vue`、`AppIcon.vue` 等受限文件改动需批准（`CONVENTIONS.md:85-87`）；③ 路由参数用 UUID `id` 而非编号 `code`（深链稳定）。

---

## 1. 壳层（Shell）现状与新增页面接线

### 1.1 单壳结构：所有受保护页面共用 `DefaultLayout`

- 路由只有一个布局节点：`/` 挂 `DefaultLayout`，其余页面是它的 `children`（`router/routes.ts:13-16`、`:167-168`）；`/login` 是唯一布局外页面（`routes.ts:7-12`）。
- 布局是固定三段式：248px 侧边栏 + 顶栏 + 可滚动 `<main>` + 状态栏；外层网格 `min-w-[1280px]`、`min-h-[720px]`、`h-dvh`（`DefaultLayout.vue:47-61`）。
- 页面标题来自路由 `meta.title`：`DefaultLayout.vue:16` 计算 `pageTitle`，`AppTopbar.vue:22-26` 展示面包屑，`guards.ts:59-63` 同步 `document.title`——**新页面只写 `meta.title`，壳层零改动**。
- 全局常驻组件挂在 `App.vue:15-20`（`PageProgress`、`CommandPalette`、`ToastHost`），仅非 `public` 路由启用（`App.vue:11-19`）；路由切换自动回顶（`DefaultLayout.vue:19-22`）。

### 1.2 侧边栏：数据驱动 + 权限过滤

- `AppSidebar.vue:23-151` 定义 `navGroups`，当前 6 组：工作台 / 空间与设备 / 机器 / 课表与事件 / 资源 / 管理；每项 `{ name, label, icon, to, permission? }`（`:15-21`）。
- 渲染前按权限过滤并丢弃空分组（`visibleGroups`，`:153-160`）——**加新入口只需加一条 item**。
- 高亮：`route.name === item.name || route.meta.activeName === item.name`（`:162`）；详情/分析页用 `meta.activeName: 'tickets'` 保持「工单」高亮（先例 `schedules/:id`，`routes.ts:78-81`、`:99`）。
- 侧栏摘要卡只展示房间/设备数（`AppSidebar.vue:182-190`，数据 `stores/shell.ts:32-60`），与工单无关。

### 1.3 顶栏 / 通知中心 / 状态栏

- 顶栏 = 面包屑 + 全局搜索（⌘K，`ui.commandPaletteOpen`）+ API 文档 + 通知中心 + 头像（`AppTopbar.vue:20-64`；通知组件 `:54`；store `stores/ui.ts:5-7`）。
- 通知中心现状见 §4；状态栏每 30s 探测 `/readyz` 展示 PG/Redis/API 延迟/实例（`AppStatusBar.vue:8-33`、`:44-52`），工单不需要改动。

### 1.4 路由与守卫

- 路由表平铺在 `routes.ts:6-168`，每页约 4 行：`path / name / 懒加载 component / meta{title, permission, activeName?}`（房间范例 `:35-46`；`account` 无权限 `:149-154`；兜底 `:161-166`）。
- 守卫（`router/guards.ts:12-44`）：① `meta.public` 放行，登录页已登录回跳（`:17-25`）；② 未认证先 `restore()`，失败带 `redirect` 去登录（`:27-33`）；③ 每次进入受保护页 `refreshProfile()`（60s TTL，`stores/auth.ts:24`、`:99-108`）保证权限变更即时生效（`:35-36`）；④ `meta.permission` 不满足跳 `forbidden`（`:38-41`）；⑤ 401 刷新失败清会话回登录并带 `redirect`（`:46-57`）。
- 结论：通知深链 `→ 未登录 → 登录 → 回到 /tickets/:id` 已由现有守卫覆盖，无需新逻辑。

### 1.5 权限与 store

- 权限常量唯一事实来源（前端侧）：`utils/permissions.ts:6-28`（21 个，与后端 `crates/domain/src/rbac.rs:10-32`、`ALL.len()==21`（`:95`）对齐）；`PermissionCode` 自动派生（`:30`）。
- 鉴权：`can/canAny`（`stores/auth.ts:32-38`）、`roles/displayName`（`:26-29`）、`refreshProfile`（`:99-108`）；toast 反馈 `stores/toast.ts:25-37`。
- 工单列表/详情状态放页面本地 `ref`（同 `RoomsView.vue:31-44`）；跨页共享的待办角标放新 store（§4.2），**不改** `auth/shell/ui/toast`（`CONVENTIONS.md:85-87` 受限）。

### 1.6 新增一个一级页面：精确步骤（工单照此执行）

| # | 文件 | 改动 | 受限 |
| --- | --- | --- | --- |
| 1 | `src/utils/permissions.ts:6-28` | 加 4 个 `ticket*` 常量 | 是，需 Lead |
| 2 | `src/features/roles/permissionGroups.ts:10-34` | `GROUP_LABELS` 加 `ticket: '工单'`、`GROUP_ORDER` 加 `'ticket'`（否则角色编辑页落到「其他」，`:48`） | 建议 Lead 知情 |
| 3 | `src/components/ui/AppIcon.vue` | 加 `ticket` / `chart` 图标；`AppIcon.vue:11-26` 起是 `v-if` 图标链，未知 name 不渲染任何图元，不加则图标不可见 | 是，需 Lead |
| 4 | `src/router/routes.ts` | 加 3 条 child route（§5.1） | 是，需 Lead |
| 5 | `src/components/layout/AppSidebar.vue:23-151` | 新增「运维」分组 2 项 | 是，需 Lead |
| 6 | `src/components/search/CommandPalette.vue:42-178` | （可选）加 `nav-tickets` 跳转项，`permission: PERMISSIONS.ticketRead`，已有权限过滤（`:170-178`） | 否，建议 P0 做 |
| 7 | `src/views/TicketsView.vue` 等 + `src/features/tickets/*` | 页面与组件（§5.4） | 否 |
| 8 | `docs/CONVENTIONS.md:100-121` | 文档表格补行（实施时） | 文档 |

**接线要点**：`meta.title` 决定顶栏/浏览器标题；`meta.permission` 决定守卫拦截；`meta.activeName` 决定侧栏高亮；`name` 用于所有 `router.push({ name })`（现有页面全按 name 跳转，如 `RoomsView.vue:130`、`MachineDetailView.vue:234`）。四项缺一不可。

### 1.7 工单需要独立布局吗？——不需要

**结论：P0 复用 `DefaultLayout`，不新增 layout。**

- 最大最复杂的页面（1447 行排课工作台 `views/ScheduleBoardView.vue`）同样是 `DefaultLayout` 的 child（`routes.ts:66-70`），页面内自带工具栏/视图切换（`features/scheduleBoard/BoardToolbar.vue`）——「页面级复杂 chrome」在本架构由页面内部消化。
- 布局提供全站一致的导航/搜索/通知/状态栏，工单页对这几项全都要，没有一项需要替换；固定 1280 宽（`DefaultLayout.vue:48`、`main.css:33`）与工单列表/详情的桌面密度匹配。
- 独立布局要复制 `pageTitle`、⌘K、通知、状态栏、守卫接入（`DefaultLayout.vue:16-43`、`AppTopbar.vue:20-64`），并让 `App.vue:11-19` 的 `workspaceShell` 判断复杂化，纯负收益。

| 只有这类场景才值得新布局（P1/P2 再评估） | 实现方式 |
| --- | --- |
| 值班大屏 / TV wall：无侧栏顶栏、深色、只读轮播、大字号 | `/` 之外的独立路由（参考 `routes.ts:7-12` 登录页）或 `meta.layout` 分支 |
| 接单台 / handler console：常驻队列 + 详情 + 键盘操作 | 优先做成 `DefaultLayout` child 的两栏页面；确需全屏再加 focus 布局 |
| 匿名报修表单（教室扫码）：无 Session、不走权限守卫 | `meta.public` 独立轻量页，类似 `LoginView.vue` |

---

## 2. 可复用页面模式（写工单前先抄这些）

### 2.1 列表页：`RoomsView.vue` / `AuditView.vue` / `AnnouncementsView.vue`

- 骨架：`PageHeader`（标题/描述/actions）→ 筛选工具栏 → 卡片内表格 → `PaginationBar`（`RoomsView.vue:175-345`；统一骨架 `CONVENTIONS.md:32-56`）。
- 分页/状态约定：`PAGE_SIZE=20`（接口上限 100，`CONVENTIONS.md:74`）、`page/total/loading/error`（`RoomsView.vue:24`、`:31-44`；`AuditView.vue:15-24`）。
- 三态必须齐全（`CONVENTIONS.md:73`）：加载用 `animate-pulse` 骨架行（`RoomsView.vue:291-293`、`AuditView.vue:62-73`）；空用 `EmptyState`，区分「有筛选」与「无数据」（`RoomsView.vue:295-314`）；错误用红色提示条 + 重试（`RoomsView.vue:211-218`、`AuditView.vue:133-139`）。
- 403 文案单独提示（`RoomsView.vue:76-78`、`AuditView.vue:52-54`）；搜索 `useDebounceFn(..., 320)` 且筛选变化回第 1 页（`RoomsView.vue:94-108`）。
- 行点击进详情 + 操作列 `@click.stop`（`RoomsView.vue:234-240`、`:263-286`）；危险操作走 `ConfirmDialog`（`:334-344`，`CONVENTIONS.md:76`）。
- 状态 tabs 用 `FilterTabs`（`AnnouncementsView.vue:160`；`FilterTabs.vue:8-21`，count 可选 `:37-40`）；下拉筛选用 `.select` + `optgroup`（`AuditView.vue:97-121`），选项可抽纯 TS 常量（`features/audit/auditOptions.ts:12-106`，工单的 kind/severity 选项照此写）。

### 2.2 详情页：`MachineDetailView.vue` / `RoomDetailView.vue`

- 独立路由 + 顶部返回链接（`MachineDetailView.vue:217-225`；`RoomDetailView.vue:325-330`；规则 `CONVENTIONS.md:80`）。
- 404/403 分开：`isNotFound` → 整块 `EmptyState`（`MachineDetailView.vue:227-236`；`RoomDetailView.vue:368-378`）；其他错误 → 顶部红色条 + 重试（`MachineDetailView.vue:279-285`）。
- `PageHeader` 的 `badge` 放状态胶囊（`MachineDetailView.vue:242`），右侧放刷新/自动刷新/实时流状态（`:252-277`）；指标卡横排 `MetricCard`（`:310`、`RoomDetailView.vue:394`）。
- 两栏 `grid-cols-12`：左 8 主内容、右 4 上下文（`MachineDetailView.vue:314-416`）；`InfoList` 字段（`:343-345`）、`PageSection` 分块（`:328`、`:371`）、关联跳转小胶囊（`:346-367`）——工单右栏「关联房间/设备/机器」照此做。
- JSON 用 `JsonBlock`（`:328-338`）、token/序列号用 `CopyText`，禁止 `v-html`（`CONVENTIONS.md:81-83`）。
- 路由参数变化要重置并重载（`MachineDetailView.vue:198-204`），避免 `/tickets/A → /tickets/B` 复用组件残留旧状态。

### 2.3 总览 / 分析页面模式

- KPI 卡：`StatCard`（`features/overview/StatCard.vue:17-80`，含 `tone/badge/loading`）或 `MetricCard`（`features/areas/MetricCard.vue:1-50`），布局 `grid grid-cols-4 gap-4`（`OverviewView.vue:147-149`）。
- 聚合数据可抽 Pinia store、页面只编排：`stores/overview.ts:34-118`（并发 `Promise.all` + 权限可选请求 `optional()` `:47-48`、`:70-81`；`requestToken` 防竞态 `:39`、`:96`）。工单分析数据量小，P0 页面本地 `load()` 即可，不建 store。
- 自动刷新 `useInterval`（`OverviewView.vue:104-108`、`composables/useInterval.ts:3-22`）；分析页 P0 只做进入加载 + 手动刷新。
- **纯 CSS 图表**（无图表库）：横条复用 `ProgressBar.vue:26-40`（`width: value%`）或自写 div；分组柱状用 `flex items-end` + 每柱 `:style="{ height: pct + '%' }"`，参考 `UsageBar.vue:31-56` 的 `role="progressbar"` 无障碍写法；颜色用现有 token（`main.css:10-22`）与 `StateBadge` tone 对齐（`StateBadge.vue:16-24`）。
- 日期范围（7/30/90 天）用 `FilterTabs` 切换并重算。

### 2.4 实时流：`features/machines/useMachineStream.ts`

- 浏览器 `EventSource` 不能带 Authorization，底层 fetch + ReadableStream（`useMachineStream.ts:8-13`；同款 presence 流 `api/index.ts:631-671`、`:627-671`）。
- 断线 3s 重连（`:31-39`），401 先触发一次普通请求走 axios 刷新令牌（`:69-75`），`onScopeDispose` 清理（`:79-89`）；使用方用 `useDebounceFn(400ms)` 折叠刷新（`MachineDetailView.vue:150-159`、`MachinesView.vue:187-194`）。
- **工单 P0 不做 SSE**：进入加载 + 操作后刷新足够；P1 若后端提供 `/admin/tickets/stream`，复用上述模式与 `after_id` 补发约定。

### 2.5 UI kit 清单（直接组合，不新增组件库）

| 组件 | 用途 / 关键点 | 引用 |
| --- | --- | --- |
| `PageHeader` + `PageSection` + `InfoList` | 页头 actions；详情分块；字段列表（`columns=1/2`、`mono`） | `PageHeader.vue:1-33`、`PageSection.vue:1-29`、`InfoList.vue:1-20` |
| `AppButton` + `IconButton` | 主/次/危险按钮；行内图标操作 | `AppButton.vue:1-56`、`IconButton.vue:1-30` |
| `StateBadge` | 状态/严重级胶囊，tone 7 种、`pulse` | `StateBadge.vue:4-24` |
| `EmptyState` + `ConfirmDialog` | 空/错误/404；危险操作确认 | `EmptyState.vue:1-29`、`ConfirmDialog.vue:1-47` |
| `AppModal` + `FormField` | 新建/编辑表单（`.input/.select/.textarea`） | `AppModal.vue:1-60`、`FormField.vue:1-24` |
| `FilterTabs` + `PaginationBar` + `SearchInput` | 状态 tabs（带 count）/ 分页 / 搜索 | `FilterTabs.vue:8-21`、`PaginationBar.vue:1-33`、`SearchInput.vue:1-28` |
| `JsonBlock` + `CopyText` | 关联 payload；编号/序列号复制 | `JsonBlock.vue:1-36`、`CopyText.vue:1-60` |
| `ProgressBar` + `MetricCard` | 分析条形 / KPI 卡 | `ProgressBar.vue:1-40`、`MetricCard.vue:1-50` |

### 2.6 格式、反馈与小工具

- 时间：`formatRelative`（列表更新时间）、`formatDateTime`/`formatFullDateTime`（详情/SLA）、`localTime`（`utils/format.ts:39-147`）；短 ID `shortId`（`:157-160`）、百分比 `formatPercent`（`:115-118`）；禁止 `new Date().toString()`（`CONVENTIONS.md:78-79`）。
- toast：`useToastStore().success/error`（`stores/toast.ts:25-37`）；轮询/防抖：`useInterval`（`composables/useInterval.ts:3-22`）、`useDebounceFn`（`:3-21`）；状态映射抽纯 TS 便于单测（`features/devices/deviceMeta.ts:11-40`、`features/announcements/announcementStatus.ts:10-26`、`features/audit/auditOptions.ts:108-126`）——工单写成 `features/tickets/ticketMeta.ts`。

---

## 3. API 层与类型生成

### 3.1 分层事实

- `src/api/index.ts` 是唯一请求出口，按资源分组（`api.listRooms`、`api.createSchedule`…，`api/index.ts:255-602`）；页面禁止直接 axios（`CONVENTIONS.md:21-23`）。
- 每条方法 = `unwrap<T>(http.get/post/patch/delete(...))`（`api/index.ts:237-244`），响应类型来自 `schema.ts`；`src/api/schema.ts` 自动生成勿手改（`schema.ts:1-2`），由 `docs/openapi.json` 经 `docs/openapi-types.py` 生成（说明 `:1-9`，主逻辑 `:66-106`）。
- 手写参数在 `src/api/types.ts`：`PageParams`（`:23-26`）、各资源列表参数（`:28-111`）、`PresenceEventParams`（`:91-94`）、探针/聚合（`:113-145`）。
- 错误：`ApiError {status, code}`（`api/index.ts:136-146`）、`isForbidden`（`:148-150`）、`isNotFound`（`:152-154`）、`errorMessage`（`utils/errors.ts:1-4`）；后端错误体 `{ error: { code, message } }`（`schema.ts:250-252`、`api/index.ts:224-244`）。
- 令牌：access token 内存、refresh token 按「记住」存 localStorage/sessionStorage（`api/session.ts:1-73`）；401 单飞刷新并重放（`api/index.ts:179-222`），页面无感。

### 3.2 后端加接口后，前端必须同步的清单

```bash
# 1) 后端实例导出最新规范 → 生成 schema
curl -s http://127.0.0.1:8080/api-docs/openapi.json -o frontend/docs/openapi.json
cd frontend && python3 docs/openapi-types.py && ./node_modules/.bin/oxfmt src/api/schema.ts

# 2) 手工维护（生成物覆盖不到）：types.ts（TicketListParams…）、api/index.ts（listTickets…）、
#    utils/permissions.ts（ticket:*）、features/roles/permissionGroups.ts

# 3) 验证
./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json && ./node_modules/.bin/vitest run
```

- `docs/openapi.json` 必须由后端真实运行实例导出（`CONVENTIONS.md:26`），前端不能手改；`docs/api-contract.md` 是同一规范的易读清单，一起刷新供评审。
- 新增字段保持 snake_case（与 `schema.ts` 一致），展示层再转中文；不要手写响应类型。
- 若后端沿用 `{ items, page, page_size, total }` 分页（`api/index.ts:604-624` 的 `*PageWire` 兼容层即为此类结构），列表页可直接套现有模式。

---

## 4. 通知中心与「我的待办工单」

### 4.1 现状（必须原样保留的行为）

- 壳层每 30s 强制刷新：`DefaultLayout.vue:40-43` 调 `shell.bootstrap(true)`；`bootstrap` 并发 `loadSummary()` 与 `refreshPresence()`（`shell.ts:75-94`），后者拉 `listDeviceEvents({limit:12})`（`:62-73`）。
- 已读：`localStorage['rn:presence:seen-id']` 存最大事件 id（`shell.ts:8-25`）；`unreadCount = presenceEvents.filter(id > lastSeenId).length`（`:43-45`）；首次加载视为已读（`:66-69`）；`markAllSeen/markSeen`（`:96-106`）。
- 菜单：徽标显示 unread（`NotificationsMenu.vue:54-60`），打开时刷新（`:18-21`），点击 → `router.push({name:'device-detail', params:{serial}})`（`:35-39`），失败/无权限静默（`shell.ts:70-72`）。

### 4.2 推荐扩展：新增独立 store + 菜单内分组（不改 shell）

原则：**不动 `shell.ts` 与 presence 的 seen 键**，避免两类通知生命周期耦合。新文件 `src/stores/ticketTodo.ts`：

```ts
// imports 略（defineStore/computed/ref、api、Ticket、auth、PERMISSIONS）
const SEEN_KEY = 'rn:tickets:seen-at' // 与 presence 的 seen-id 完全独立

export const useTicketTodoStore = defineStore('ticketTodo', () => {
  const todos = ref<Ticket[]>([])
  const lastSeenAt = ref(readSeenAt()) // localStorage 读取，失败回退 epoch
  const unreadCount = computed(() => todos.value.filter((t) => Date.parse(t.updated_at) > Date.parse(lastSeenAt.value)).length)

  async function refresh() {
    const auth = useAuthStore()
    const me = auth.user?.id
    if (!auth.can(PERMISSIONS.ticketRead) || !me) return // 无权限静默
    if (document.visibilityState === 'hidden') return // 后台标签页不轮询
    try {
      // status 只支持单值（task-1 §6.2）：未关闭 = open + in_progress 两次轻量查询
      const [open, doing] = await Promise.all(
        ['open', 'in_progress'].map((status) => api.listTickets({ assignee_id: me, status, page_size: 10 })),
      )
      todos.value = [...(open.items ?? []), ...(doing.items ?? [])]
    } catch {
      // 403 / 网络失败静默，不打断 presence 通知
    }
  }

  function markAllSeen() {
    lastSeenAt.value = new Date().toISOString()
    try { localStorage.setItem(SEEN_KEY, lastSeenAt.value) } catch { /* 内存兜底 */ }
  }

  return { todos, unreadCount, refresh, markAllSeen }
})
```

`NotificationsMenu.vue`（受限文件，需 Lead 批准）最小改动：

1. 引入 store；`onMounted` 拉一次 + `useInterval(() => tickets.refresh(), 60_000)`（组件常驻顶栏，菜单关闭也更新角标；**不复用 30s 壳层时钟**）。
2. 徽标合计 `unread + tickets.unreadCount`，保留 `9+` 截断（`NotificationsMenu.vue:54-60`）。
3. 面板加「我的待办」分组：`code + title + 状态胶囊 + formatRelative(updated_at)`，点击 `tickets.markAllSeen()` 后 `router.push({ name: 'ticket-detail', params: { id: ticket.id } })`（写法同 `:35-39`）。
4. 原「设备在线事件」分组不动，两组各有「全部已读」，空态文案分开（「暂无设备在线事件」/「暂无待办工单」）。

| 备选方案 | 改动面 | 评价 |
| --- | --- | --- |
| A. 扩展 `NotificationsMenu`（推荐） | 1 个受限 layout 文件 + 新 store | 一个铃铛、改动集中；需 Lead 批准 |
| B. 扩展 `shell.ts` | 受限 store + 菜单 | 复用 30s bootstrap，但 presence/ticket 耦合，回滚风险大 |
| C. 顶栏新增独立「工单待办」铃铛 | `AppTopbar`（受限）+ 新组件 + 新 store | 不碰现有通知，但两个铃铛、信息分散 |

### 4.3 不破坏现有行为的关键约束

- seen 键独立：`rn:tickets:seen-at` vs `rn:presence:seen-id`（`shell.ts:8`）；权限独立：无 `ticket:read` 不发请求（`auth.can`，`stores/auth.ts:32-34`）；失败静默（同 `shell.ts:70-72` 策略）。
- 轮询成本：现状每 30s 3 个请求（`shell.ts:47-73`）；新增后每 60s +2 个轻量查询（open/in_progress 各一，`page_size=10`；后端加 `active` 参数后降为 1 个），后台标签页跳过；对比状态栏/总览的 30s 轮询（`AppStatusBar.vue:28-33`、`OverviewView.vue:104-108`）仍属低开销。
- 深链：通知项必须携带 **ticket id（UUID）**，不用 `code`；守卫自动补登录与权限校验（`guards.ts:17-41`）。

---

## 5. P0 落地方案

### 5.1 路由（`src/router/routes.ts`）

```ts
{
  path: 'tickets',
  name: 'tickets',
  component: () => import('@/views/TicketsView.vue'),
  meta: { title: '工单', permission: PERMISSIONS.ticketRead },
},
{
  path: 'tickets/insights',
  name: 'ticket-insights',
  component: () => import('@/views/TicketInsightsView.vue'),
  meta: { title: '工单分析', permission: PERMISSIONS.ticketRead, activeName: 'tickets' },
},
{
  path: 'tickets/:id',
  name: 'ticket-detail',
  component: () => import('@/views/TicketDetailView.vue'),
  meta: { title: '工单详情', permission: PERMISSIONS.ticketRead, activeName: 'tickets' },
},
```

- 静态 `tickets/insights` 与动态 `:id` 要区分，声明顺序静态在前（先例 `schedules/list` 先于 `schedules/:id`，`routes.ts:71-82`）。
- 新建工单用页面内 `AppModal`，不新增 `/tickets/new` 路由。

### 5.2 侧边栏「运维」分组（`AppSidebar.vue:23-151`）

加在「资源」（`:92-117`）之后、「管理」（`:118-150`）之前：

```ts
{
  label: '运维',
  items: [
    { name: 'tickets', label: '工单', icon: 'ticket', to: '/tickets', permission: PERMISSIONS.ticketRead },
    { name: 'ticket-insights', label: '工单分析', icon: 'chart', to: '/tickets/insights', permission: PERMISSIONS.ticketRead },
  ],
},
```

权限过滤、空分组剔除、详情高亮均自动生效（`:153-162`）。

### 5.3 权限常量（`src/utils/permissions.ts:6-28`）

```ts
ticketRead: 'ticket:read',       // 列表/详情/分析
ticketCreate: 'ticket:create',   // 人工建单
ticketHandle: 'ticket:handle',   // 认领、评论、状态流转
ticketManage: 'ticket:manage',   // 改派、取消任意工单、自动规则配置
```

- 与后端 task-1 的 `rbac::permissions` 一一对应（后端 `ALL.len()` 将从 21 增加，`crates/domain/src/rbac.rs:32-95`）。
- 角色编辑页权限来自 `/admin/permissions`（`api/index.ts:545`），只需补分组标签（`permissionGroups.ts:10-34`），无需硬编码权限项。
- 页面按钮显隐 + 后端 403 双保险：`auth.can(PERMISSIONS.ticketHandle)`（模式 `RoomsView.vue:46`、`:180`）。

### 5.4 文件与组件拆分

| 文件 | 职责 |
| --- | --- |
| `views/TicketsView.vue` | 列表页编排：状态 tabs、筛选、分页、新建 modal（抄 `RoomsView.vue` + `AnnouncementsView.vue:160`） |
| `views/TicketDetailView.vue` | 详情页编排：详情、时间线、评论、状态操作（结构抄 `MachineDetailView.vue`） |
| `views/TicketInsightsView.vue` | 分析页编排：日期范围 + KPI + CSS 图表（布局抄 `OverviewView.vue:147-149`） |
| `features/tickets/ticketMeta.ts` | 状态/严重级（severity）/类型（kind）/来源文案与 tone、`availableActions()` 状态机、停留时长判定（纯 TS，单测重点） |
| `features/tickets/TicketFilters.vue` | 状态 tabs + 严重级/类型/区域/处理人/对象/关键词/时间范围 |
| `features/tickets/ticketFilters.ts` | 筛选 ↔ URL query 的纯函数（parse/serialize），页面与单测共用 |
| `features/tickets/TicketTable.vue` | 表格 + 骨架 + `EmptyState` + 行点击 |
| `features/tickets/TicketStatusBadge.vue` | 状态胶囊（含「超期停留」派生），薄封装 `StateBadge` |
| `features/tickets/TicketTimeline.vue` | 事件时间线（`ticket_events` 的 created/assigned/status_changed/commented/occurred，系统条目区分）+ 评论输入（`.textarea`，纯文本） |
| `features/tickets/TicketContextCard.vue` | 关联房间/设备/机器 + 最新在线/健康 + 跳转入口（抄 `MachineDetailView.vue:346-367`） |
| `features/tickets/TicketStatsCards.vue` | 分析 KPI：新增/已解决/未关闭/平均处理时长/超时（`MetricCard` 组合） |
| `features/tickets/TicketTrendChart.vue` | 新增 vs 关闭 日柱状图（CSS，`flex items-end` + 高度百分比） |
| `features/tickets/TicketKindBars.vue` | 类型（kind）分布 / 房间 TOP5 / 处理人负载（CSS 横条，复用 `ProgressBar`） |
| `features/tickets/TicketCreateModal.vue` | 新建表单（`AppModal + FormField`，提交 loading、成功 toast + 刷新） |
| `stores/ticketTodo.ts` | 通知「我的待办」角标（§4.2） |

不建 `stores/tickets.ts`：列表数据是页面局部状态；分析页后续多视图复用再抽。

### 5.5 API 方法与类型（`api/index.ts` + `api/types.ts`，最终以 task-1 契约为准）

```ts
// src/api/types.ts
export interface TicketListParams extends PageParams {
  status?: TicketStatus; kind?: TicketKind; severity?: TicketSeverity
  assignee_id?: string; room_id?: string; area_id?: string
  keyword?: string; from?: string; to?: string
}
export interface TicketStatsParams { from?: string; to?: string; days?: 7 | 30 | 90 }
export interface TicketCommentInput { note: string }
export interface TicketUpdateInput {
  status?: TicketStatus; assignee_id?: string | null; severity?: TicketSeverity
  title?: string; description?: string; resolution?: string | null; note?: string
}

// src/api/index.ts —— 新分组 "---- 工单 ----"
listTickets: (params: TicketListParams = {}) =>
  unwrap<TicketPage>(http.get('/admin/tickets', { params })),
getTicket: (id: string) =>
  unwrap<TicketDetail>(http.get(`/admin/tickets/${encodeURIComponent(id)}`)), // { ticket, events }
createTicket: (input: CreateTicketRequest) => unwrap<Ticket>(http.post('/admin/tickets', input)),
/** PATCH 承担状态流转/转派/改级；resolved 必带 resolution（task-1 §6.2） */
updateTicket: (id: string, input: TicketUpdateInput) =>
  unwrap<Ticket>(http.patch(`/admin/tickets/${encodeURIComponent(id)}`, input)),
assignTicket: (id: string, input: AssignTicketRequest) =>
  unwrap<Ticket>(http.post(`/admin/tickets/${encodeURIComponent(id)}/assign`, input)),
commentTicket: (id: string, input: TicketCommentInput) =>
  unwrap<TicketEvent>(http.post(`/admin/tickets/${encodeURIComponent(id)}/comments`, input)),
/** 分析聚合：后端 P0 清单暂无此端点，需 Lead 决策（§7） */
ticketStats: (params: TicketStatsParams = {}) =>
  unwrap<TicketStats>(http.get('/admin/tickets/stats', { params })),
```

- 响应类型全部来自再生成的 `schema.ts`（`Ticket/TicketPage/TicketDetail/TicketEvent/TicketStats/...`），不手写。
- 错误约定：列表 403 → 明确「缺少 ticket:read」（`RoomsView.vue:76-78`）；详情 404 → `EmptyState`（`MachineDetailView.vue:227-236`）；流转 409 → `toast.error('工单状态已变化', …)` 并重载详情。

### 5.6 文案与 tone 映射（`ticketMeta.ts`）

| 维度 | 值 → 文案 / tone |
| --- | --- |
| status | `open` 待处理（amber）、`in_progress` 处理中（sky）、`resolved` 已解决（emerald）、`closed` 已关闭（slate）、`cancelled` 已取消（slate） |
| severity | `P0` 紧急（rose）、`P1` 高（amber）、`P2` 普通（sky）、`P3` 低（slate）；task-3 原型的 priority 只是展示别名 |
| kind | `machine_health` 机器健康、`device_offline` 设备离线、`schedule_delivery` 推送失败、`device_presence_delivery` 在线推送失败、`storage_transfer` 分发失败、`image_verify` 镜像校验失败、`agent_config_downlink` 配置下发失败、`security` 安全、`manual` 人工（原型 category 为中文别名） |
| 来源 | `created_by` 为 null → 系统自动（副文案带 `source_type/source_id`）；非空 → 人工 |
| 超时 | 后端 P0 无 `due_at`（task-1 DDL 未含 SLA），先用「停留时长 = now - created_at」在超阈值时提示；SLA/超时留 P1 |

状态机（按钮显隐按此，后端校验）：`open→in_progress→resolved→closed`、`open/in_progress→cancelled`；`resolved→in_progress`（重开）需 `ticket:manage`；`resolved` 必带 resolution，`closed` 须先 resolved。

### 5.7 列表页交互

- 状态 tabs：全部/待处理/处理中/已解决/已关闭/已取消（`FilterTabs`），切换回第 1 页；筛选行含严重级（P0–P3）、类型（kind）、区域/房间、处理人（「我的」快捷）、搜索（标题/编号/描述）、时间范围（7/30 天），搜索走 `useDebounceFn(320)`；KPI 条（待处理/处理中/已解决/超时）若后端 counts 缺失则不显示数字。
- **URL 同步**（参考 `MachinesView.vue:151-163`、`:213-219`）：`status/severity/kind/assignee_id/q/page` 写入 query，进入页面回填，默认不写 `page=1`——保证通知/分享链接、浏览器后退可用。
- 表格列（1280 预算见 §6.5）：编号（含来源小字）｜标题（`line-clamp-1`，附评论数）｜状态｜严重级｜类型｜对象（房间·设备）｜处理人｜更新时间；行点击进详情，行内只留「认领/开始」等高频动作。
- 危险操作（取消）走 `ConfirmDialog`（`danger`）；空态区分「无数据」与「筛选无结果」（`RoomsView.vue:295-308` 模式）。

### 5.8 详情页

- 顶部 `← 返回工单列表`（`RouterLink :to="{name:'tickets'}"`，模式 `MachineDetailView.vue:217-225`）。
- `PageHeader`：标题 `code + title`、badge 状态、右侧按状态与权限显隐的操作按钮组。
- 信息区 `InfoList`：类型（kind）/严重级/状态/上报人（created_by）/处理人（assignee_id）/创建/更新/解决时间/发生次数（occurrence_count）/解决说明（resolution）；描述纯文本（`<p class="whitespace-pre-wrap">`，禁 `v-html`，`CONVENTIONS.md:83`）。
- 右栏：`TicketContextCard`（房间/设备/机器 + 最新在线/健康 + 详情跳转）+ 关联快照/事件 `JsonBlock`（视后端返回）。
- 时间线 + 评论：`TicketDetail.events`（created/assigned/unassigned/status_changed/commented/occurred）；`POST /comments {note}` 成功 toast 后重载详情（不做乐观更新，避免与状态机冲突）。
- 流转：`PATCH /tickets/:id {status, note}`；`resolved` 前用 `AppModal + FormField` 收集 `resolution`，`closed` 须先 resolved；409/403 用 `errorMessage` 提示并重载；取消走 `ConfirmDialog`（danger）。
- 权限/存在性：`isForbidden` → `EmptyState('缺少工单权限')`；`isNotFound` → `EmptyState('工单不存在')`（分开提示）。

### 5.9 分析页

- `PageHeader` + `FilterTabs`（近 7/30/90 天）+ 刷新。
- KPI：新增工单 / 已解决 / 未关闭 / 平均处理时长（MTTR）/ 超时工单。
- 趋势：新增 vs 关闭 按日分组柱状图（CSS），Y 轴最大值为 0 时显示空态；类型（kind）横条、房间/设备 TOP5、处理人负载。
- “洞察”文本由纯函数从同一份 stats 推导（如「近 7 天机器健康类占 42%，集中在 3 间房间」），随区间刷新，不硬编码。
- **契约缺口**：task-1 后端 P0 清单（列表/详情/PATCH/comments/assign）没有统计端点；分析页需 Lead 决策新增 `GET /admin/tickets/stats`（推荐），否则降级为「列表 KPI + ≤100 条前端聚合」并标注“近似统计”，禁止分页拉全量。

### 5.10 通知中心改动点

| 文件 | 改动 |
| --- | --- |
| `stores/ticketTodo.ts` | 新建（§4.2） |
| `NotificationsMenu.vue:15-42` | 引入 store、`onMounted` + 60s `useInterval`、独立「全部已读」 |
| 同上 `:54-60` | 徽标 = presence unread + ticket unread，保留 `9+` |
| 同上 `:62-106` | 面板顶部加「我的待办工单」分组，点击 `router.push({ name: 'ticket-detail', params: { id } })` |
| `AppTopbar.vue` | 不改（除非选备选 C） |

### 5.11 测试

- 单元（vitest jsdom，`vitest.config.ts:8-12`）：
  - `features/tickets/__tests__/ticketMeta.spec.ts`：五种状态、P0–P3 严重级、kind 中文映射兜底、`availableActions()` 状态机、停留时长/超时判定（固定 `now`）。
  - `features/tickets/__tests__/ticketFilters.spec.ts`：query 解析/序列化（空值剔除、非法 `page` 回退、状态白名单）。
  - `features/tickets/__tests__/ticketStats.spec.ts`：`buildDailySeries` 缺日补零、MTTR 忽略未解决、TOP5 排序稳定、超时计数。
  - `src/__tests__/ticketTodo.spec.ts`：mock `@/api`（模式 `App.spec.ts:57-73`）+ `MemoryStorage`（`:11-37`）；断言 seen 键独立、unreadCount 随 `updated_at` 变化、无 `ticket:read` 不发请求。
- 端到端（`e2e/tickets.spec.ts`，复用 `e2e/vue.spec.ts:3-12` 的 `signIn`）：登录 → 侧栏「工单」→ 列表；新建工单出现在首行；点击行进 `/tickets/:id` 详情；提交评论后时间线出现；认领/开始处理后状态胶囊变化、返回列表同步；无权限账号访问 `/tickets` 落 `/forbidden`。
- 命令：`./node_modules/.bin/vitest run`、`./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json`、`./node_modules/.bin/playwright test e2e/tickets.spec.ts`（需 dev server 5174 + 后端 8080；类型检查/构建由 Lead 统一跑，`CONVENTIONS.md:8-14`）。

### 5.12 交付顺序与工作量

| 阶段 | 内容 | 人天 |
| --- | --- | --- |
| P0-1 | 权限常量/图标/路由/侧边栏/命令面板（4 个受限文件需审批） | 0.5 |
| P0-2 | API 方法 + 类型 + 契约联调（依赖 task-1） | 1.0 |
| P0-3 | 列表页（tabs/筛选/URL/分页/新建） | 2.0 |
| P0-4 | 详情页（状态机/时间线/评论/关联上下文） | 2.5 |
| P0-5 | 分析页（KPI/CSS 图表/洞察） | 1.5 |
| P0-6 | 通知中心「我的待办」+ 深链 | 0.5 |
| P0-7 | vitest + Playwright + 回归 | 1.5 |
| **合计** | | **≈9.5**（通知/分析后置可压到 7） |

---

## 6. 风险与边界

1. **URL/筛选状态**：现有列表页多数筛选只存内存（`RoomsView.vue:31-44` 仅 `area_id` 读 query `:36`），刷新即丢；工单必须做 query 同步（先例 `MachinesView.vue:151-163`），否则通知深链/分享/后退体验差；路由参数变化要重置状态（`MachineDetailView.vue:198-204` 模式）。
2. **403 兜底双层**：守卫先按 `meta.permission` 拦（`guards.ts:38-41` 跳 `/forbidden`），页面内接口 403 仍要提示（权限可能在打开期间被回收，`auth.ts:24` 有 60s TTL）；详情页必须区分 403/404，避免泄露权限外工单存在性。
3. **通知轮询成本**：现有壳层每 30s 3 个请求（`DefaultLayout.vue:40-43`、`shell.ts:47-73`）；工单待办独立 60s + `visibilityState` 跳过 + 权限预判 + 失败静默 + 菜单打开即刷新（§4.3），不要把工单请求塞进壳层时钟。
4. **深链时序**：通知 → 会话过期 → 登录 → 回跳 `/tickets/:id` 由 `guards.ts:17-33` + `safeRedirect` 覆盖；无 `ticket:read` 会落 `/forbidden?from=...`（`guards.ts:40`、`routes.ts:155-160`）；通知项用 UUID 而非 `code`。
5. **1280px 表格密度**：布局最小宽 1280（`DefaultLayout.vue:48`、`main.css:33`），减 248 侧栏与 `px-7`（56）后可用约 976px；列表最多 8 列，编号/状态/严重级/时间窄列、标题弹性，类型与来源合并，解决说明/评论摘要进详情。沿用 `table-th/table-td`（`main.css:111-117`），短字段 nowrap、标题 truncate。
6. **详情页一致性**：`CONVENTIONS.md:80` 要求独立路由、禁止抽屉；`features/scheduleBoard/BlockDrawer.vue` 只是画布内临时编辑，不是详情先例。工单详情不要用 `AppModal`/抽屉，否则刷新、分享、通知深链、撤销都会退化。
7. **XSS / 富文本**：评论、描述、来源信号一律纯文本（`CONVENTIONS.md:83` 禁 `v-html`）；未来若支持 Markdown 需引入 sanitizer（新依赖，单独评审）。
8. **自动工单噪音**：健康/离线/投递失败规则会产生重复工单，前端默认「待处理」+「我的」快捷筛选、按更新时间排序、默认近 7 天窗口；同对象去重由后端负责（task-1）。
9. **契约依赖**：接口/权限码目前都不存在，§5.5 是前端期望形状，最终以 task-1 与真实 OpenAPI 为准；`schema.ts` 必须再生成，禁止手写。
10. **受限文件与角色页一致性**：`utils/permissions.ts`、`router/routes.ts`、`AppSidebar.vue`、`NotificationsMenu.vue`、`AppIcon.vue` 均需 Lead 批准（`CONVENTIONS.md:85-87`）；后端加 `ticket:*` 后若 `permissionGroups.ts` 不补分组，角色编辑页落到「其他」（`:48`）。
11. **实时性边界**：P0 不做工单 SSE（task-1 也只把 SSE 列为 P1：`/tickets/stream?after_id`），进入加载 + 操作后刷新 + 60s 通知轮询已够；P1 复用 fetch-SSE 模式（`api/index.ts:631-671`）并处理 401 刷新（`useMachineStream.ts:69-75`）。

---

## 7. 待确认问题（给 Lead / 后端）

1. 工单主键与编号：路由用 `id`（UUID）还是 `code`（T-YYYYMMDD-####）？建议 `id`，`code` 仅展示/搜索。
2. 列表参数：`status` 只支持单值 + `assignee_id`，「我的待办」目前需 open/in_progress 两次查询；是否加 `active`/多值 status 与 counts。
3. 统计端点：task-1 P0 未含 `GET /admin/tickets/stats`，是否补；MTTR 定义（创建→解决，是否排除取消）；P0 无 `due_at`，超时用停留时长还是补 SLA 字段。
4. 通知中心是否批准改 `NotificationsMenu.vue`（方案 A）；否则是否接受独立铃铛（方案 C）。
5. 图标：`AppIcon.vue` 增加 `ticket`、`chart` 两个线性图标（Lead 改）。
6. 自动工单来源信号（`metadata`/`source_type/source_id`）在详情里用 `JsonBlock` 还是纯文本；task-3 原型的 category/priority 与后端 kind/severity 的映射是否统一。
