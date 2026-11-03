# 管理端前端约定

本目录是 `room_node_backend` 的管理控制台：Vue 3 + Vite + Tailwind CSS 4 + Pinia，
UI 结构与交互模式沿用 QEStudio `frontend` 模板，数据层对接本仓库的 `/api/v1` 管理 API。

## 运行与验证

依赖已复制到 `node_modules/`（本机 pnpm store 在沙箱里只读，**不要执行 `pnpm install`**）。

```bash
./node_modules/.bin/vite --host 127.0.0.1 --port 5174   # 开发服务器，/api 代理到 127.0.0.1:8080
./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json  # 只做类型检查（并发安全）
./node_modules/.bin/vite build                            # 生产构建
./node_modules/.bin/oxfmt src                             # 格式化（提交前由 Lead 统一跑）
```

- 后端已经在 `127.0.0.1:8080` 运行，开发代理见 `vite.config.ts`。
- 开发账号：`admin` / `Admin@12345`（仅本地 `.env`，不要写进代码/README）。
- 数据库是共享开发库，创建测试数据请加 `dev-` 前缀，验证后尽量删除。

## API 层

- `src/api/index.ts`：唯一的请求出口，按资源分组（`api.listRooms`、`api.createSchedule`…）。
  需要新接口时**不要让页面直接 axios**，先看是否已有方法；确实缺失时联系 Lead 添加。
- `src/api/schema.ts`：从 `docs/openapi.json` 自动生成，字段与后端 snake_case 完全一致，勿手改。
  刷新方式：`curl -s http://127.0.0.1:8080/api-docs/openapi.json -o docs/openapi.json && python3 docs/openapi-types.py`。
- `src/api/types.ts`：列表查询参数、会话、探针等手写类型。
- 错误统一是 `ApiError`（有 `status`/`code`），页面用 `errorMessage(error, fallback)` 转文案；
  403 可以用 `isForbidden(error)` 单独提示“缺少权限”。
- 鉴权/刷新令牌、401 重放都在 `src/api/index.ts` 里，页面不用关心。

## 页面结构

统一使用模板风格，页面骨架：

```vue
<div class="mx-auto max-w-[1440px] px-7 py-5">
  <PageHeader title="房间" description="...">
    <template #actions>
      <SearchInput v-model="keyword" placeholder="搜索房间" />
      <AppButton v-if="auth.can(PERMISSIONS.roomManage)" icon="plus" variant="primary" @click="openCreate">
        新建房间
      </AppButton>
    </template>
  </PageHeader>

  <!-- 筛选工具栏 -->
  <div class="mt-4 flex items-center justify-between gap-3">…</div>

  <!-- 表格 -->
  <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
    <table class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th pl-5">名称</th>
          …
        </tr>
      </thead>
      <tbody>
        <tr class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50">
          <td class="table-td pl-5">…</td>
        </tr>
      </tbody>
    </table>
    <PaginationBar :page="page" :total-pages="totalPages" :total="total" @change="loadPage" />
  </section>
</div>
```

要求：

- 页面只做取数/编排，复杂块拆到 `src/features/<domain>/` 下的组件。
- 必须处理三种状态：加载中（骨架/`animate-pulse` 行）、空状态（`EmptyState`）、错误（红色提示条 + 重试）。
- 列表页支持分页（`page_size` 20；接口上限 100）与搜索/筛选；筛选变化时回到第 1 页。
- 操作按钮用 `auth.can(PERMISSIONS.xxx)` 控制显隐；后端仍会返回 403，页面要能兜底提示。
- 危险操作（删除、撤销、重放、发布）必须走 `ConfirmDialog`。
- 新建/编辑表单放在 `AppModal` 里，用 `FormField` + `.input` / `.select` / `.textarea`；提交按钮 loading。
- 时间展示用 `@/utils/format` 里的函数（`formatRelative` / `formatDateTime` / `formatFullDateTime` / `localTime`），
  大小用 `formatBytes`，短 ID 用 `shortId`；不要直接 `new Date().toString()`。
- 详情页用独立路由（`rooms/:id` 等），不要用抽屉；标题下有返回链接。
- JSON（审计 detail、manifest、payload）用 `JsonBlock` 展示，不要 `JSON.stringify` 塞在 `<p>` 里。
- token / serial / sha256 等用 `CopyText` 提供复制按钮。
- **禁止 `v-html`**：公告正文只以纯文本/`<pre>` 展示，避免 XSS（后端虽会清洗，前端仍不做第二解释器）。
- 全部文案用中文，语气与现有页面一致；图标用 `AppIcon`，缺图标先联系 Lead 加。
- 不引入新依赖；不修改 `vite.config.ts`、`package.json`、`src/api/**`、`src/router/**`、
  `src/components/ui/**`、`src/components/layout/**`、`src/stores/{auth,shell,ui,toast}.ts`、
  `src/assets/main.css`、`src/utils/**`（需要改动先找 Lead）。

## 可复用组件

`src/components/ui/`：`AppButton`、`AppIcon`、`AppModal`、`ConfirmDialog`、`CopyText`、`EmptyState`、
`FilterTabs`、`FormField`、`IconButton`、`InfoList`、`JsonBlock`、`PageHeader`、`PageSection`、
`PaginationBar`、`ProgressBar`、`SearchInput`、`StateBadge`、`ToastHost`、`ToggleSwitch`。

- `StateBadge`：通用状态胶囊，`tone` = emerald / amber / rose / slate / sky / violet / accent。
- `IconButton`：图标按钮，`variant` = ghost / outline，适合表格行操作。
- `useInterval` / `useDebounceFn` 在 `src/composables/`。
- 反馈用 `useToastStore().success/error`；权限用 `useAuthStore().can/roles` 与 `PERMISSIONS`。

## 当前页面与文件归属

| 页面 | 路由 | View | feature 目录 |
| --- | --- | --- | --- |
| 总览 | `/` | `OverviewView.vue` | `features/overview/`（Lead） |
| 区域 | `/areas`、`/areas/:id` | `AreasView.vue`、`AreaDetailView.vue` | `features/areas/` |
| 房间 | `/rooms`、`/rooms/:id` | `RoomsView.vue`、`RoomDetailView.vue` | `features/rooms/` |
| 设备 | `/devices`、`/devices/:serial` | `DevicesView.vue`、`DeviceDetailView.vue` | `features/devices/` |
| 机器总览 | `/machines`、`/machines/:serial` | `MachinesView.vue`、`MachineDetailView.vue` | `features/machines/` |
| 在线事件 | `/device-events` | `DeviceEventsView.vue` | `features/deviceEvents/` |
| 课表 | `/schedules`、`/schedules/:id` | `SchedulesView.vue`、`ScheduleDetailView.vue` | `features/schedules/` |
| 触发事件 | `/events` | `EventsView.vue` | `features/events/` |
| 镜像 | `/images`、`/images/:id` | `ImagesView.vue`、`ImageDetailView.vue` | `features/images/` |
| 存储 | `/storage` | `StorageView.vue` | `features/storage/` |
| 节点配置 | `/agent-configs` | `AgentConfigsView.vue` | `features/agentConfigs/` |
| 工单 | `/tickets`、`/tickets/:id` | `TicketsView.vue`、`TicketDetailView.vue` | `features/tickets/` |
| 工单分析 | `/tickets/insights` | `TicketInsightsView.vue` | `features/tickets/` |
| 自动规则 | `/tickets/rules` | `TicketRulesView.vue` | `features/tickets/`（`ticketRuleMeta.ts`、`TicketRuleModal.vue`） |
| 可见范围 | `/tickets/scopes` | `TicketScopesView.vue` | `features/tickets/` |
| 巡检计划 | `/tickets/inspections` | `TicketInspectionsView.vue` | `features/tickets/`（`InspectionPlanModal.vue`、`InspectionRunsPanel.vue`、`inspectionMeta.ts`） |
| 用户 | `/users` | `UsersView.vue` | `features/users/` |
| 角色 | `/roles` | `RolesView.vue` | `features/roles/` |
| 审计 | `/audit` | `AuditView.vue` | `features/audit/` |
| 公告 | `/announcements` | `AnnouncementsView.vue` | `features/announcements/` |
| 账户 | `/account` | `AccountView.vue` | `features/account/` |

路由、侧边栏与权限点已经接好；页面内不要再改路由。

## 后端契约参考

- `docs/api-contract.md`：全部 60+ 接口的方法、查询参数、请求/响应类型。
- `docs/openapi.json`：原始 OpenAPI 3.1 规范。
- `README.md`（仓库根目录）：业务语义、权限点、投递/重建规则。
