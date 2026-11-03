# 排课工作台前端调研与三候选界面交互规格

> 范围：`frontend/`（Vue3 + Vite + Tailwind4 + Pinia）。只读调研，未改动 `frontend/src/`。
> 目的：回答「能不能在一个界面里可视化编排所有房间」，给出 A 周网格 / B 资源时间轴 / C 全房间编排矩阵
> 三个候选形态的可实现规格，以及推荐的产品形态、组件树与分阶段落地路径。
> 后端契约以 `docs/schedule-design.md`、`README.md:323-410`、`docs/openapi.json` 为准；
> 本文不重复后端设计，只记录**影响前端交互的事实**。

---

## 0. 结论摘要（给 Lead）

1. **最优组合不是三选一**：A 当默认视图（看与改周模板）、B 当「某一天跨房间调课」视图（改真实日期，落单日覆盖）、
   C 当「开学/换季批量铺课」视图（框选 + 批量填充）。三者共享同一草稿、筛选、冲突引擎与保存条。
2. **最大的坑不是画布，是语义**：后端只有「房间 → 课表 → 周模板 / 单日覆盖 / 物化实例」，**没有节次实体，也没有教师/课程实体**
   （`docs/schedule-design.md:20`）。所以 A/C 的「周」必须是**模板周**（改一次影响整个生效区间），
   B 的「周几」必须是**具体日期**（任何改动都要写单日覆盖）。这条必须在 UI 上明示，否则用户会误删整学期课。
3. **前端可以做到「一个界面排全部房间」，但代价是 N 次请求**：现有 API 没有跨房间聚合
   （`frontend/src/api/index.ts:347-395`），8 房间需要 3 + 2×8 = 19 个 GET。先用并发池 + 逐行骨架兜住，
   由 Lead 的后端任务决定是否补聚合接口（`boardOccurrences` / `boardSlots`）。
4. **推荐落地**：把 `/schedules` 改造成排课工作台（`name: schedules` 不变），原列表挪到 `/schedules/list`，
   `/schedules/:id` 保留为「单房间精细维护」。不新增侧边栏项，不动权限点。
5. **工作量粗估：约 12–13 人天前端**（A 只读 2.5d → 编辑闭环 3d → B 3d → C 2.5d → 回归 1.5d），
   不含后端接口与联调等待；若后端只提供现有接口，前端通过 N 请求也能交付，性能可接受（8 房间量级）。

---

## 1. 现状盘点

### 1.1 路由与入口

| 路由 | name | View | 权限 | 引用 |
| --- | --- | --- | --- | --- |
| `/schedules` | `schedules` | `SchedulesView.vue` | `schedule:read` | `frontend/src/router/routes.ts:66-70` |
| `/schedules/:id` | `schedule-detail` | `ScheduleDetailView.vue` | `schedule:read`，`activeName: schedules` | `frontend/src/router/routes.ts:72-76` |
| `/events` | `events` | `EventsView.vue` | `schedule:event:read` | `frontend/src/router/routes.ts:78-82` |

- 侧边栏只有「课表」一个入口，位于「课表与事件」分组，`to: '/schedules'`（`frontend/src/components/layout/AppSidebar.vue:74-89`）。
- 全局搜索导航项 `nav-schedules` 指向 `{ name: 'schedules' }`（`frontend/src/components/search/CommandPalette.vue:79-85`），
  搜索到的具体课表跳 `schedule-detail`（`frontend/src/components/search/CommandPalette.vue:213-220`）。
  → 把 `/schedules` 改成工作台后，前者落到工作台、后者仍落详情，行为自然成立，无需改搜索结果。
- 路由守卫按 `meta.permission` 拦截，403 由页面兜底（`frontend/src/router/guards.ts:38-40`、`frontend/src/utils/permissions.ts:18-19`）。

### 1.2 页面与 feature 组件

**列表页 `/schedules`（`SchedulesView.vue`）**

- 所有房间下拉筛选 + 分页 20 条，无日期、区域、关键词筛选（`SchedulesView.vue:23`、`:35`、`:52-72`、`:177-195`）。
- 表格列：课表名 / 房间 / 起止日期 / 时区 / 状态 / 更新时间 / 操作（`SchedulesView.vue:197-267`）。
- 新建课表走 `ScheduleFormModal`：**必须先选房间、填名称、起止日期**（`SchedulesView.vue:94-116`、
  `features/schedules/ScheduleFormModal.vue:52-75`、`:77-113`），创建成功后直接跳详情（`SchedulesView.vue:110`）。
- 空库时展示 `EmptyState`（`SchedulesView.vue:269-280`），文案是「还没有课表」，没有「一次为全部房间生成课表」的入口。

**详情页 `/schedules/:id`（`ScheduleDetailView.vue`）**

- 从上到下四块：基本信息 `InfoList` → 周模板 `WeeklySlotEditor` → 单日覆盖 `ScheduleExceptionPanel` →
  排课实例 `OccurrencePanel`（`ScheduleDetailView.vue:290-345`）。
- 周模板保存 = 全量替换并重建未来实例（`ScheduleDetailView.vue:180-191`、`api/index.ts:366-369`）。
- 单日覆盖：在面板点「新增覆盖」→ 弹窗里选日期、选 `cancel`/`replace`、replace 还要逐行加时段
  （`features/schedules/ScheduleExceptionPanel.vue:65-80`、`:120-142`、`features/schedules/ScheduleExceptionFormModal.vue:28-67`、`:77-100`）。
- 排课实例查询区间默认今天起 7 天（`ScheduleDetailView.vue:56-57`），表格只读展示（`features/schedules/OccurrencePanel.vue:124-170`）。
- 周模板编辑器是**表格逐行编辑**：`addRow()` 复制上一行星期、默认 09:00–10:00（`features/schedules/WeeklySlotEditor.vue:55-59`、
  `features/schedules/scheduleForm.ts:61-70`），删除、还原、保存都在表格上方（`:96-128`）。
- 前端校验只覆盖「同一天时段不重叠」（`features/schedules/scheduleForm.ts:93-130`，重叠判定 `:111-128`）。

**嵌入位置**

- 房间详情页：`RoomSchedulesPanel` 展示该房间的课表列表 + 近 7 天实例（`views/RoomDetailView.vue:454`、
  `features/rooms/RoomSchedulesPanel.vue:52-68`），列表项可直接进详情（`:125-142`）。
- 区域详情页：区域下前 100 张课表 + 课表事件流（`views/AreaDetailView.vue:73-76`、`:230-287`、`:448-455`、
  `features/areas/ScheduleEventFeed.vue:1-98`）。
- 事件页：按房间/类型/时间筛选事件，无课表画布（`views/EventsView.vue:40-43`、`:101-130`）。

### 1.3 API 层（课表相关）

| 方法 | 签名/路径 | 引用 |
| --- | --- | --- |
| `listSchedules` | `GET /admin/schedules?room_id&page&page_size`，仅 `room_id` 一个业务筛选 | `frontend/src/api/index.ts:348-349`、`frontend/src/api/types.ts:47-49` |
| `getSchedule` | `GET /admin/schedules/:id` | `api/index.ts:351-352` |
| `createSchedule` | `POST /admin/schedules` | `api/index.ts:354-355` |
| `updateSchedule` | `PATCH /admin/schedules/:id` | `api/index.ts:357-358` |
| `deleteSchedule` | `DELETE /admin/schedules/:id`（级联删实例） | `api/index.ts:360-361` |
| `weeklySlots` | `GET /admin/schedules/:id/weekly-slots` | `api/index.ts:363-364` |
| `replaceWeeklySlots` | `PUT /admin/schedules/:id/weekly-slots`，**全量替换** | `api/index.ts:366-369` |
| `exceptions` | `GET /admin/schedules/:id/exceptions?from&to` | `api/index.ts:371-374` |
| `upsertException` | `PUT /admin/schedules/:id/exceptions/:date` | `api/index.ts:376-379` |
| `deleteException` | `DELETE /admin/schedules/:id/exceptions/:date` | `api/index.ts:381-382` |
| `occurrences` | `GET /admin/schedules/:id/occurrences?from&to` | `api/index.ts:384-386` |
| `roomOccurrences` | `GET /admin/rooms/:id/occurrences?from&to` | `api/index.ts:389-391` |
| `rebuildSchedule` | `POST /admin/schedules/:id/rebuild` | `api/index.ts:394-395` |

关键类型（`frontend/src/api/schema.ts`，自动生成，**禁止手改**）：

- `Schedule`：`room_id/room_name/start_date/end_date/timezone/is_active/updated_at`（`schema.ts:688-701`）。
- `WeeklySlot`：`id/course_name/teacher/note/day_of_week/start_time/end_time/room_id?/room_name?`（`schema.ts:940-951`）。
- `Occurrence`：`occurrence_date/start_local/end_local/start_at/end_at/source/course_name/teacher`（`schema.ts:546-561`）。
- `ScheduleException`：`exception_date/kind(cancel|replace)/slots[]`（`schema.ts:742-763`）。
- `NewWeeklySlot`：**已带可选 `room_id`（调课到其他房间）**（`schema.ts:536-544`），但现有前端编辑器从不传
  （`features/schedules/scheduleForm.ts:132-141`）——这是「跨房间」在数据层唯一的现成抓手。
- `OccurrenceParams` 只有 `from/to`（`api/types.ts:86-89`），没有房间/区域/多 ID 批量参数。

### 1.4 权限

- `schedule:read` / `schedule:manage` / `schedule:event:read` / `schedule:event:manage`（`frontend/src/utils/permissions.ts:18-21`）。
- 页面用 `auth.can(PERMISSIONS.scheduleManage)` 控制写入口（`SchedulesView.vue:42`、`ScheduleDetailView.vue:63`、
  `WeeklySlotEditor.vue:104-126`），后端仍会 403 兜底（`router/guards.ts:38-40`）。
- **没有房间级/区域级细粒度授权**：拿到 `schedule:manage` 就能改所有房间。
  → 工作台做「只读模式」只需判断一个权限点，删/拖/框选全部隐藏即可。

### 1.5 设计系统与可用画布尺寸

全局 token（`frontend/src/assets/main.css:10-22`）：`canvas #f5f6f8`、`surface #fff`、`line #e6e8ee`、
`ink #15171c`、`muted #6f7787`、`accent #3a5bd9`、`accent-soft #eef2ff`；正文 14px，组件字号集中在
10.5–13.5px（`.input/.table-th/.table-td` 见 `main.css:88-121`）。**没有提供 chart/时间轴/拖拽类 token**。

布局骨架（`frontend/src/layouts/DefaultLayout.vue:48`）：`grid-cols-[248px_minmax(0,1fr)]`、`min-w-[1280px]`；
顶栏 56px（`components/layout/AppTopbar.vue:21`）、底部状态栏 28px（`components/layout/AppStatusBar.vue:56`）。
页面内容 `mx-auto max-w-[1440px] px-7 py-5`（`frontend/docs/CONVENTIONS.md:37`）。

据此计算出工作台的**真实可用画布**（后续所有尺寸都以这两行为基准）：

| 指标 | 1280×800（最小支持） | 1440×900（设计稿基准） |
| --- | --- | --- |
| 主内容宽（减 248 侧栏） | 1032 | 1192 |
| 内容可用宽（减 px-7×2） | **976** | **1136** |
| 主区高（减 56 顶栏、28 状态栏） | 716 | 816 |
| 减 py-5 后的内容高 | 676 | 776 |
| 减 PageHeader(~55) + 工具栏(56) + 保存条(52) | **≈500** | **≈600** |

> 结论：工作台画布高度只有 **500–600px**，且 `min-width: 1280px` 明说移动端不做（`main.css:33`）。
> 所有行高/列宽必须按这个高度倒推，不能照搬教务系统的 1080p 大画布。

### 1.6 后端事实（决定前端能做什么，逐条都有代码依据）

| # | 事实 | 依据 | 对前端的直接影响 |
| --- | --- | --- | --- |
| B1 | 课表挂在房间；**同一房间可以有多份课表** | `README.md:347-348`、`docs/schedule-design.md:59-73` | 行/列头必须定义「主课表」选择规则，否则同一房间会渲染重叠的两套课 |
| B2 | 周模板 = ISO 星期 + 本地钟面 `HH:MM`；**同一 weekday 内后端禁止重叠** | `docs/schedule-design.md:74-90`、`crates/domain/src/validation.rs:436-472`（`"同一星期几的时段不能重叠"` :470） | 单房间模板冲突在保存时会被后端 400 拒绝，前端必须在拖拽阶段就阻止 |
| B3 | **跨房间/跨课表冲突后端完全不校验**（没有全局冲突表，也没有教师实体） | `crates/domain/src/validation.rs:436-472`、`docs/schedule-design.md:20` | 冲突检测是纯前端计算；教师冲突只能按字符串精确匹配做「警告」 |
| B4 | 周模板/单日覆盖可带 `room_id` 覆盖 = **调课到其他房间** | `docs/schedule-design.md:74-90`、`README.md:349`、`schema.ts:536-544` | 「跨房间移动」在数据层可行（写覆盖），但现有 UI 不暴露；语义要跟后端确认（见 §6-Q6） |
| B5 | 保存周模板 = **全量替换**，并重建**未来**实例 | `frontend/docs/api-contract.md:617-627`、`README.md:396` | 必须做草稿/撤销；保存按课表串行，失败要能保留未保存项 |
| B6 | 重建窗口 `SCHEDULE_HORIZON_DAYS` 默认 30 天（1–366） | `crates/api/src/config.rs:57`、`:186-188` | B 的「真实日期」只能可靠浏览今天起 ~30 天；更远的日期没有实例 |
| B7 | 实例只重写未来（`start_at > now()`），历史实例不补 | `README.md:351-353` | B 往前翻历史可能为空，要有「历史实例未生成」说明 |
| B8 | 时间点统一 UTC，展示用 `start_local/end_local` + 区域时区；模板 `HH:MM` 是区域时区下的钟面时间 | `README.md:328-330`、`:360-372` | 画布时钟用 `HH:MM` 字符串，**不要用 `start_at` 手动换算**；跨时区区域不能共用一根时间轴 |
| B9 | 没有节次/学期/教师/课程实体 | `docs/schedule-design.md:20`、`schema.ts` 的 `WeeklySlot.teacher/note` 均为字符串 | A/C 的「节次」只能是前端常量模板；教师冲突只能字符串比对 |
| B10 | 单日覆盖 replace = **整天替换**，cancel = **整天停课** | `docs/schedule-design.md:91-105`、`README.md:350` | B 里拖动一节课，实际要写「当天整套时段」的 replace；不能只改一节 |

**实测开发库**（只读探测 `127.0.0.1:8080`，admin 账号）：区域 12 / 房间 12 / 课表 1（周模板为空）。
区域与房间都存在**同名多条**（多个 `agent config test area/room`），课表名为 `test`。
→ 工作台的分组与排序不能只按名称，必须 `area_id + name + code` 稳定排序，行头展示 `区域/房间 · 编码`；
线上真实目标按题目给的 **2 区域 × 8 房间** 估算，开发库的 12 房间可作为「更大数据量」的压力参考。

### 1.7 现有交互痛点（★ = 对 A/B/C 的影响）

| # | 痛点 | 代码依据 | A | B | C |
| --- | --- | --- | --- | --- | --- |
| P1 | **必须先建课表才能排课**：列表页新建表单强制房间/名称/日期，空库无「为所有房间初始化」入口 | `SchedulesView.vue:94-116`、`ScheduleFormModal.vue:52-113`、`scheduleForm.ts:34-44` | ★ | ★ | ★ |
| P2 | **周模板只能逐条 addRow**：表格行内编辑，默认 09:00–10:00、星期沿用上一行；无拖拽、无批量、无复制 | `WeeklySlotEditor.vue:55-85`、`scheduleForm.ts:61-70` | ★ | | ★ |
| P3 | **无跨房间视图**：一房一页，详情页三块纵向堆叠；看 8 房间要开 8 个标签页 | `ScheduleDetailView.vue:290-345`、`SchedulesView.vue:197-267` | ★ | ★ | ★ |
| P4 | **无冲突可视化**：只有保存时前端/后端对「同一课表同一 weekday」校验；跨房间、跨课表、教师全无提示 | `scheduleForm.ts:111-128`、`crates/domain/src/validation.rs:436-472` | ★ | ★ | ★ |
| P5 | **单日覆盖入口深**：详情页底部面板 → 弹窗 → 选日期 → 选 cancel/replace → 逐行加时段 | `ScheduleExceptionPanel.vue:65-80`、`:120-142`、`ScheduleExceptionFormModal.vue:28-100` | | ★ | ★ |
| P6 | **保存即全量替换**：没有草稿、撤销、重做；误删一行保存后无法回退（只能重建/手改） | `ScheduleDetailView.vue:180-191`、`api/index.ts:366-369` | ★ | ★ | ★ |
| P7 | **无日期/区域/关键词筛选**：列表只能按房间过滤，无法「看本周」「看 P3 区域」 | `SchedulesView.vue:177-195`、`api/types.ts:47-49` | ★ | ★ | ★ |
| P8 | **状态不可见**：房间列表没有「是否有课表/是否启用/有效期」列，只能进房间详情才知道 | `views/RoomsView.vue:222-230`、`RoomSchedulesPanel.vue:52-68` | ★ | ★ | ★ |
| P9 | **教师/课程每次手输**：无课程库、无历史候选，复制粘贴能力为零 | `WeeklySlotEditor.vue:202-231`、`schema.ts:940-951` | ★ | ★ | ★ |
| P10 | **多课表歧义**：同房间多份课表时，列表/房间面板都平铺展示，没有「当前生效」概念 | `README.md:347-348`、`RoomSchedulesPanel.vue:118-146` | ★ | ★ | ★ |
| P11 | **API 无跨房间聚合**：`listSchedules` 只按 room；`occurrences` 只能按课表或房间逐个取 | `api/index.ts:348-349`、`:384-395` | ★ | ★ | ★ |
| P12 | **无拖拽基础设施**：项目禁止新增依赖，`AppIcon` 也没有拖拽/撤销/网格类图标 | `CONVENTIONS.md:85`、`AppIcon.vue` 图标清单 | ★ | ★ | ★ |
| P13 | **URL 无状态**：只有 `/schedules/:id`，无法分享「P3 区域 2026 秋季第 10 周」 | `router/routes.ts:66-76` | ★ | ★ | ★ |

---

## 2. 可复用资产

### 2.1 直接复用（不改代码）

| 资产 | 用途 | 引用 |
| --- | --- | --- |
| `PageHeader` / `PageSection` | 页头、区块容器 | `components/ui/PageHeader.vue`、`PageSection.vue` |
| `AppButton` / `IconButton` | 工具栏、行操作 | `components/ui/AppButton.vue:1-60`、`IconButton.vue` |
| `AppModal` + `FormField` | 新建/批量填充/冲突确认弹窗（`size="lg"/"xl"`） | `components/ui/AppModal.vue:1-120`、`FormField.vue` |
| `ConfirmDialog` | 删除课块、整天停课、批量覆盖、放弃修改 | `components/ui/ConfirmDialog.vue:1-55` |
| `EmptyState` | 无房间/无课表/筛选无结果 | `components/ui/EmptyState.vue:1-30` |
| `FilterTabs` | 三视图切换（周网格/时间轴/编排矩阵） | `components/ui/FilterTabs.vue:1-45` |
| `StateBadge` | 冲突/来源/只读/课表状态胶囊（tone 7 种） | `components/ui/StateBadge.vue:1-50` |
| `ToggleSwitch` | 密度切换、只看冲突、显示覆盖 | `components/ui/ToggleSwitch.vue` |
| `SearchInput` | 房间/课程搜索 | `components/ui/SearchInput.vue` |
| `PaginationBar` | 保留给 `/schedules/list` 旧列表 | `components/ui/PaginationBar.vue` |
| `InfoList` / `CopyText` | 抽屉里的课表元信息/ID 复制 | `components/ui/InfoList.vue`、`CopyText.vue` |
| `useToastStore` | 保存/拖拽失败/批量结果反馈 | `stores/toast.ts:1-40` |
| `useAuthStore().can` + `PERMISSIONS` | 只读降级 | `stores/auth.ts:27-38`、`utils/permissions.ts:18-21` |
| `useDebounceFn` / `useInterval` | 搜索防抖 / 失效缓存轮询（可选） | `composables/useDebounceFn.ts`、`useInterval.ts` |
| `utils/format`：`WEEKDAY_LABELS`/`weekdayLabel`/`addDays`/`toDateInput`/`today`/`formatDateTime`/`shortId` | 周几、日期运算、格式化 | `utils/format.ts:121-158` |
| `errorMessage` / `isForbidden` | 错误文案与 403 提示 | `utils/errors.ts:1`、`api/index.ts:141` |
| `scheduleForm.ts` 的校验/映射 | 面板与弹窗复用；但工作台需要扩展为 `SlotDraft + id` | `features/schedules/scheduleForm.ts:1-172` |

**视觉可复用的既有页面结构**：`schedules.png` 的空列表卡片、`room-detail.png` 的双列信息卡、`area-detail.png`
的分块事件流，都证明「`rounded-xl border border-line bg-surface` + 10.5–13.5px 字号」是当前控制台的统一语言，
工作台应在同一语言内做**高密度画布**，不要引入大圆角/彩色填充的大卡片。

### 2.2 需要新建的组件（建议目录 `frontend/src/features/scheduleBoard/`）

```
features/scheduleBoard/
├── boardTypes.ts              # BoardBlock/BoardRoom/BoardEdit/Conflict/ViewMode 等（不与 schema.ts 混用）
├── periodTemplate.ts          # 节次常量（见 §3.0），slot→节次映射、节次起止时间
├── roomColors.ts              # 房间色板（8 色，hue 稳定 hash by room.id）
├── boardStore.ts              # 见 §4.4
├── components/
│   ├── BoardToolbar.vue       # 视图切换/日期导航/区域/房间/只看冲突/刷新/新建
│   ├── BoardCanvasShell.vue   # 统一滚动容器、粘性表头、空/错/加载态分发
│   ├── WeekGridView.vue       # A 方案
│   ├── WeekGridBlock.vue
│   ├── ResourceTimelineView.vue  # B 方案
│   ├── TimelineBlock.vue
│   ├── AllRoomMatrixView.vue  # C 方案
│   ├── MatrixBlock.vue
│   ├── ScheduleBlockChip.vue  # 三视图共用视觉（颜色/冲突/覆盖/只读）
│   ├── BlockDetailDrawer.vue  # 右侧课程详情 + 编辑
│   ├── BlockEditModal.vue     # 新建（预填房间/星期/节次/时间）
│   ├── BatchFillModal.vue     # C 框选批量填充
│   ├── CreateSchedulesModal.vue  # 为「无课表房间」批量初始化
│   ├── ConflictSummary.vue    # 工具栏冲突计数 + 弹层列表
│   ├── BoardSaveBar.vue       # 未保存条：撤销/重做/放弃/保存
│   ├── BoardLegend.vue        # 颜色/冲突/覆盖图例
│   └── BoardEmptyState.vue    # 三种空态（无房间/无课表/筛选无结果）
└── composables/
    ├── useBoardData.ts        # 取数 + 缓存 + 并发池 + 逐行刷新
    ├── useBoardDraft.ts       # 草稿、脏标记、撤销重做、保存合并
    ├── useBoardConflicts.ts   # 冲突索引与实时预览
    ├── useBlockDrag.ts        # Pointer Events 拖拽/改时长（无依赖）
    ├── useMatrixSelection.ts  # C 的框选/复制粘贴
    └── useBoardKeyboard.ts    # 快捷键分发
```

配套改动（需 Lead 同意，属于跨任务/受保护文件）：

- `views/ScheduleBoardView.vue`（新页面）+ `views/SchedulesListLegacy` 或把现 `SchedulesView.vue` 移到 `views/SchedulesListView.vue`；
- `router/routes.ts`：`/schedules` 指向工作台、新增 `/schedules/list`（避免旧的 `name: schedules` 失效）；
- `components/ui/AppIcon.vue` 补 3–4 个图标：`undo`（或复用 `rotate-ccw`）、`redo`、`grid`、`rows`、`move`；
  按 `CONVENTIONS.md:84`「缺图标先联系 Lead 加」走；
- 新增 `features/scheduleBoard/` 下文件不违反「不修改 `api/**`、`router/**`、`components/ui/**`」的约定，
  但路由与 `api/index.ts` 的改动必须由 Lead 统一做（`CONVENTIONS.md:85`）。

**与 CONVENTIONS 的冲突点（要先决策）**：`CONVENTIONS.md:80` 规定「详情页用独立路由，不要用抽屉」。
工作台的右侧抽屉是**画布内联检查器/编辑面**，不是页面级详情。两种落地方式：
(a) 推荐：新增 `BoardDrawer` 并请 Lead 在 CONVENTIONS 增补「工作台内联检查器例外」；
(b) 保守：复用 `AppModal size="xl"`（最大 `max-w-4xl`）做右偏弹窗，零规范冲突但会遮挡画布。

### 2.3 API 层新增方法建议（只给签名，不实现；由 Lead 决定是否先做）

> 现状若不加接口，前端可用 N 次请求 + 客户端拼装完成（8 房间 = 19 个 GET，可接受）。
> 下面是**能显著降低复杂度**的聚合接口，按优先级排序：

```ts
// api/types.ts 手写参数类型；返回类型复用 schema.ts 里的 Schedule/WeeklySlot/Occurrence/ScheduleException
export interface BoardQuery {
  from?: string             // YYYY-MM-DD（含）
  to?: string               // YYYY-MM-DD（含）
  area_id?: string
  room_id?: string
  include_inactive?: boolean
}

// 1) 一次拿「可见房间 + 每房主课表 + 是否有课表」，首屏只需 1 个请求
boardRooms(query: BoardQuery): Promise<{
  rooms: Array<{
    room: Room
    area: Area
    primary_schedule: Schedule | null
    schedule_count: number
    has_conflict?: boolean       // 后端能算就更好，算不了前端算
  }>
}>

// 2) 批量周模板（按 schedule_ids 拉，替代 N 次 weeklySlots）
boardSlots(params: { schedule_ids: string[] }): Promise<Record<string /*scheduleId*/, WeeklySlot[]>>

// 3) 批量单日覆盖（跨房间，用于 A/C 显示「当天被覆盖」徽标 + B 的当日编辑）
boardExceptions(params: { schedule_ids?: string[]; from: string; to: string; area_id?: string; room_id?: string }): Promise<Array<ScheduleException & { schedule_id: string; room_id: string }>>

// 4) 跨房间实例（B 视图一次拿当天全部房间）
boardOccurrences(params: { from: string; to: string; area_id?: string; room_id?: string }): Promise<Occurrence[]>

// 5) 批量保存（可选，用于 C 一次铺 8 个房间；没有就前端串行 N 次 PUT，逐个显示进度）
batchReplaceWeeklySlots(input: { schedules: Array<{ schedule_id: string; slots: NewWeeklySlot[] }> }): Promise<{ saved: string[]; failed: Array<{ schedule_id: string; error: string }> }>

// 6) 批量建课表（可选，用于「为 N 个无课表房间一键初始化」）
batchCreateSchedules(input: { rooms: Array<{ room_id: string; name: string }>; start_date: string; end_date: string; is_active?: boolean }): Promise<Schedule[]>
```

前端调用方式：`api.boardRooms(...)` 等新方法仍然只在 `api/index.ts` 里加，页面/feature 不直接用 axios
（`CONVENTIONS.md:23-29`）。**新增方法的类型如果进入 `schema.ts` 需要重新生成**（`CONVENTIONS.md:25-26`），
手写的部分放在 `api/types.ts`，返回结构复杂的建议放 `features/scheduleBoard/boardTypes.ts` 并做映射。

### 2.4 取数与缓存策略（三方案共用）

| 数据 | 来源 | 频率/缓存 | 失效 |
| --- | --- | --- | --- |
| 区域 | `listAreas({page_size:100, include_inactive:true})` | 进入页面一次，会话内缓存 | 手动刷新 |
| 房间 | `listRooms({page_size:100, area_id?})` | 一次；区域筛选可复用全量 | 手动刷新 |
| 课表 | `listSchedules({page_size:100})` 全量后客户端按 `room_id`/`is_active`/日期过滤（接口不支持日期过滤） | 一次 | 保存/新建后重拉 |
| 周模板 | `weeklySlots(scheduleId)` 逐课表 | `Map<scheduleId, WeeklySlot[]>`，TTL 60s | 该课表 PUT 成功后失效 |
| 单日覆盖 | `exceptions(scheduleId, {from,to})` 逐课表 | `Map<scheduleId|dateRange>` | 覆盖写/删后失效 |
| 实例 | `roomOccurrences(roomId, {from,to})` 逐房间（B 用；带 room 覆盖语义正确） | `Map<roomId|dateRange>`，TTL 30s | 保存模板/覆盖后失效 |

- 并发池 6；每行独立 loading（行头显示 `animate-pulse` 细条），**不要整页 loading**；
- 请求失败：该房间行标 `加载失败 + 重试`，不阻塞其他行（对齐 `RoomSchedulesPanel.vue:102-109` 的错误条样式）；
- 切换视图不重新取数：A/C 用周模板缓存，B 用实例缓存；切换区域/房间才失效；
- 请求竞态用 `requestToken` 模式（参考 `stores/overview.ts:33-41`），避免慢请求覆盖新筛选结果。

### 2.5 性能估算（8 房间 × 7 天 × 6 节 = 336 块）

| 视图 | 渲染块数（满排） | DOM 量估算 | 是否需要虚拟滚动 |
| --- | --- | --- | --- |
| A 周网格 | 336（但按格聚合后可见 chip ≈42–120） | ~600–1500 节点 | 不需要 |
| B 单日时间轴 | 8 房间 × 6 节 = 48（+覆盖幽灵 ≤96） | ~400 节点 | 不需要（12 房间 = 72 块仍 <1000 节点） |
| C 编排矩阵 | 336 块 + 336 空 cell + 42 列头 ≈ 850+ | ~2500 节点 | 不需要；>20 房间或 >3 区域时再加「按区域折叠」而不是虚拟滚动 |

- Vue 3 渲染 336 个 `<ScheduleBlockChip>` 实测量级远低于 16ms 一帧的门槛；真正的风险是**拖拽时全量重算**：
  拖拽期间只维护 `dragging` 与 `dropTarget` 两个 ref，用 `transform` 移动浮层，**不要**在 `pointermove` 里改数据/重算冲突；
  落点合法性用「目标行 × 目标时段」线性扫描（每行每天 ≤200 条上限，通常 ≤10 条）。
- 冲突索引：`Map<roomId + date/weekday, Block[]>`，按 start 排序后双指针比较，O(n log n)；
  每次全量重算 <1ms，但只在拖拽开始、放下、批量选择、保存成功后重算。
- 长列表/横向滚动用原生 `overflow-auto` + CSS `position: sticky`；C 的 42 列横向滚动时给左右边缘加
  `shadow-[inset_...]` 渐隐提示，不引入滚动条库。

---

## 3. 三方案交互规格

### 3.0 三方案共用的基础设定（先对齐语义）

**S1 视图语义（最重要）**

| 视图 | 时间维度 | 读写对象 | 保存语义 |
| --- | --- | --- | --- |
| A 周网格 | **模板周**（周一…周日） | 周模板 `weekly-slots` | 改一次影响整个生效区间；保存 = 全量替换该课表模板 + 后端重建未来实例 |
| B 资源时间轴 | **具体日期** | 排课实例 + 单日覆盖 `exceptions` | 改动只影响这一天：当天有覆盖就改覆盖，没有就新建 `kind=replace`（整天时段替换）；整天停课 = `kind=cancel` |
| C 编排矩阵 | **模板周**（周一…周日 × 节次），可切换「按日期周」只读预览 | 周模板 `weekly-slots` | 同 A，但支持跨房间批量 |

UI 明示方式：A/C 工具栏日期区显示 `模板周 · 2026 秋季（11-02 ~ 01-31 生效）`；B 显示 `2026-11-03 周二 · 真实课表`；
A/C 上任何块（含被覆盖日）都加小角标 `模板`；B 的块角标 `当天`/`覆盖`/`停课`。
**绝不允许** A/C 在「按日期周」模式下直接编辑模板（只读预览 + 提供「以此周为样板编辑模板」入口）。

**S2 节次模板（后端无节次实体，前端常量）**

```
01  08:30–10:00   02  10:10–11:40   03  13:30–15:00
04  15:10–16:40   05  16:50–18:20   06  18:30–20:00
```
- 判定：`slot.start_time ∈ [节次.start, 节次.end)` 归入该节；块内始终显示**真实时间** `08:30–09:50`；
- 跨节次（如 09:30–11:00）：A 渲染跨行块（CSS grid `grid-row` span，只跨完整覆盖的节次，半覆盖用 `…` 标记）；
  C 把块写入覆盖到的每个节次列（首列显示文字，其余列显示同色窄条，点击仍命中同一块）；
- 若某课表用法与节次模板严重不符（大量 25 分钟/夜间时段），A 退化为「按时间分带」（08:00–21:00 每小时一行），
  C 仍用节次列；这是**待确认问题 Q3**。

**S3 主课表选择（一房多课表）**

```
候选 = 该房间所有 is_active=true 且 start_date ≤ 视图日期 ≤ end_date 的课表
主课表 = 候选按 updated_at 倒序取第一
行头显示：房间名 + 主课表名 + 「共 N 份 ▾」（点开可切换本行使用的课表，仅影响本视图，不写库）
无候选（全部过期/停用）→ 该行置灰，显示「课表已过期/停用」，仍可只读展示最近一份
无任何课表 → 空行 + 「创建课表」，工具栏提供「为 N 个房间创建课表」批量入口
```
- 若后端后续加「每房唯一启用课表」约束，此规则可简化为取该唯一课表（**待确认 Q5**）。

**S4 权限与只读**

- `schedule:read` 且有 `schedule:manage`：可拖/框选/新建/删除/保存；
- 只有 `schedule:read`：画布照常渲染，隐藏拖拽手柄、框选、新建按钮、保存条；抽屉为只读；
  工具栏加 `StateBadge tone=slate「只读」` + tooltip「缺少 schedule:manage 权限」；
- 无 `schedule:read`：路由守卫已拦（`router/guards.ts:38-40`），页面不需要额外兜底。

**S5 未保存变更**

- 移动/新建/删除/批量填充/复制粘贴全部只改**本地草稿**；底部出现 `BoardSaveBar`：
  `「{n} 项未保存」 [撤销] [重做] [放弃] [保存到 {m} 张课表]`；
- 有草稿时：`beforeunload` 提示 + `onBeforeRouteLeave` 弹 `ConfirmDialog`（不要用浏览器原生 confirm）；
- 保存成功后清空历史记录、刷新受影响课表的模板/实例缓存、冲突重算；部分失败保留失败项并 toast 明细；
- 保存串行（一次一张课表）避免同时触发多次「重建未来实例」；进度文案 `保存 2/3 · P3 房间`。

**S6 冲突规则（三方案统一）**

| 冲突 | 判定 | 级别 | 视觉 |
| --- | --- | --- | --- |
| 同房间重叠 | 生效房间（`slot.room_id ?? schedule.room_id`）相同 + 同一 weekday（A/C）或同一日期（B）+ `start < other.end && end > other.start` | **阻止**：A/C 拖拽落点非法直接回弹；B 同样回弹（B 不支持同房重叠，确需两节课同室同时段时先改另一节或拆到两个房间） | 块左边框 `border-l-2 border-rose-500` + `bg-rose-50`；拖拽落点玫瑰色虚线 |
| 同房间多课表互撞 | 同房间存在 >1 张启用课表且有效期重叠 | **结构警告**（行头三角图标，点击看说明） | 行头 `StateBadge tone=amber「多课表」` |
| 同教师重叠 | `teacher.trim()` 非空且完全相等（不做模糊），同一时段不同房间 | **警告**（允许落点，但块加 amber 边框 + 冲突计数 + 列表） | `border-l-2 border-amber-500`；抽屉显示「与 P1 房间《高数》教师王老师冲突」 |
| 超出课表有效期 | 新/移动后的 weekday 或日期不在 `start_date~end_date` | **阻止保存**（允许落点，块置灰 + 角标 `超出有效期`） | 灰色斜纹覆盖 + `StateBadge tone=slate` |
| 单日覆盖覆盖模板 | 该日期存在 `cancel`/`replace` | 提示不阻止 | 模板块 45% 透明 + 斜纹；`cancel` 整天重色遮罩 + 「停课」；日期列头加徽标 |
| 跨区域时区不一致 | 选中区域（或全部房间）存在不同 IANA 时区 | **阻止 B 的跨区共轴**；A/C 仅警告 | 工具栏 amber 提示条「所选房间跨时区，时间轴按 {tz} 显示」；B 强制先选单一区域 |

- 冲突计数放工具栏：`⚠ 3`（rose 为阻止、amber 为警告，分开显示 `2 阻止 / 1 警告`）。计数**按块去重**：一个块与多个块冲突只计 1 次，级别取最高（阻止 > 警告）；点击展开列表（每条：块名 + 冲突对象 + 时间），点条目切视图/滚动并选中块；
- 拖拽实时预览：目标合法 → `accent` 描边 + `bg-accent-soft/60`；非法 → 玫瑰虚线 + `cursor: not-allowed` + 悬浮文字说明；
  教师冲突 → amber 虚线（允许放下，放下后 toast 警告一次，同块不重复骚扰）。

**S7 拖拽实现约束（无新依赖）**

- 全部用 Pointer Events（`pointerdown/move/up` + `setPointerCapture`），不引入任何拖拽库（`CONVENTIONS.md:85`）；
- 统一 `useBlockDrag.ts`：输入 `{ mode: 'move'|'resize-start'|'resize-end', block, snap, bounds, onPreview, onDrop }`，
  内部处理 4px 启动阈值、`requestAnimationFrame` 节流、边缘自动滚动（距容器边 48px 时 8–10px/帧）；
- 拖动浮层：`position: fixed` + `transform: translate3d`，`pointer-events: none`，原块 `opacity-40`；
- Esc 取消拖拽并还原；拖拽期间禁用文本选择 `select-none`，保存条显示「拖拽中…」；
- 触屏不做（`min-width: 1280`，移动端不需要）。

**S8 空/错/加载态（三方案共用外壳 `BoardCanvasShell`）**

| 状态 | 表现 |
| --- | --- |
| 首屏加载 | 行头立即渲染（名称占位）+ 画布 42/8 个 `animate-pulse` 单元格；不整页转圈 |
| 部分行加载 | 行头右侧 12px 转圈；该行块 60% 骨架 |
| 无房间 | `EmptyState`「还没有房间」+「去创建房间」跳 `/rooms` |
| 有房间无课表 | 画布保留行；空格虚线 + 行尾「创建课表」；工具栏「为 N 个房间创建课表」 |
| 有课表无课程 | 画布照常，空格提供「点击新建」提示；顶部横幅「在空白格点击即可新建，或到编排矩阵批量铺课」 |
| 加载失败 | 顶部 rose 错误条 + 重试（对齐 `RoomSchedulesPanel.vue:102-109`）；单行失败只影响该行 |
| 筛选无结果 | `EmptyState`「当前筛选没有房间」+「清除筛选」 |

---

### 3.A 方案 A · 周网格课表

**心智模型**：教务系统课表。列 = 周一…周日，行 = 节次；每格展示该时段的课块；可切全部房间/单房间/区域。

#### A-1 布局与尺寸

| 指标 | 1280×800 | 1440×900 |
| --- | --- | --- |
| 画布可用宽 | 976 | 1136 |
| 时间列宽 | 64px | 64px |
| 天列宽（7 等分） | (976−64)/7 = **130px** | (1136−64)/7 = **153px** |
| 表头行高（星期 + 日期） | 44px | 44px |
| 节次行高（6 行） | (500−44)/6 = **76px** | (600−44)/6 = **92px** |
| 格内可用宽（减 padding 8） | 122px | 145px |
| 横向滚动 | 不需要 | 不需要 |
| 纵向滚动 | 不需要（6 行 456 < 500） | 不需要 |
| 粘性 | 表头 `sticky top-0`；时间列 `sticky left-0`（为以后加「区域」分组列预留） | 同左 |

**信息密度（8 房间 × 7 天 × 5~6 节）**

- 满排 336 块；42 个格平均 8 块/格，标准块高 36px → 288px，远超 76–92px 行高；
- 因此**「全部房间」模式不能逐块平铺**，规格如下：
  - `全部房间`：每格最多显示 **2 块**（课程名 1 行 + 房间 chip），其余折叠为 `+6` 胶囊；
    点 `+N` 或点格空白 → 弹出该格课程列表（房间色点 + 课程名 + 时间，点击进抽屉）；
    卡片 hover 时提高不透明度；格内左侧画 8 条 2px 房间色带（roomColors）表示占用密度；
  - `单房间`：每格最多 1 块（后端保证同 weekday 不重叠），显示课程名 12px + 教师/时间 10px 两行；
  - `区域`（2–4 房间）：最多 3 块 + `+N`，块高 28px 紧凑版；
- 密度切换：紧凑（24px 块 / 单行）/ 标准（36px 块 / 双行），默认标准，单房间模式自动紧凑留白。

#### A-2 核心交互

| 交互 | 规格 |
| --- | --- |
| 点击空格 | 单房间/区域模式：打开 `BlockEditModal`，预填当前列 weekday + 该节次起止（默认 08:30–10:00），房间 = 当前筛选/行（区域模式弹房间选择），课程名聚焦；全部房间模式：先弹该格课程列表（`+N` 展开），底部「+ 新建」再进表单。Enter 保存、Esc 关闭；保存即写草稿（不立即请求） |
| 点击课块 | 选中 + 打开右侧 `BlockDetailDrawer`（380px）；抽屉显示房间/课表/星期/节次/真实时间/课程/教师/备注；hover 时块上浮 1px + accent 描边 |
| 拖拽移动 | 只能改 **星期 + 节次**（房间在 A 里通过抽屉「房间」下拉改，落库为 `slot.room_id` 覆盖）；吸附到目标节次起始，保留原时长；跨格拖拽实时显示落点阴影；非法落点（同房同时段已有课）→ 玫瑰虚线 + 禁止光标；放下合法 → 改草稿 + 自动打开抽屉（可关） |
| 拖拽边缘改时长 | **不支持**（节次离散，A 是表格语义）；抽屉里改开始/结束时间，块按节次重新落位；若时间跨节次，渲染跨行块 |
| 框选批量 | 不支持（框选是 C 的卖点） |
| 复制/粘贴整周 | 支持「单房间整周复制」：工具栏 `复制本周 ▾` → 目标房间（同区域）→ 生成 5×N 条草稿（默认复制周一~周五；冲突预览后确认）；也支持单块 Ctrl+C/V |
| 撤销/重做 | 支持，作用于草稿历史（Ctrl+Z / Ctrl+Shift+Z，工具栏按钮） |
| 键盘 | 方向键在块间移动选中；Shift+方向键在抽屉里微调时间；Delete 删除选中；Enter 开抽屉；Esc 关抽屉/取消拖拽；Ctrl+S 保存 |
| 冲突预览 | 拖拽时校验目标格的同房重叠（单房间模式几乎必冲突 → 落点直接禁止）；全部房间模式允许不同房间同格共存 |
| 单击 vs 双击 | 单击 = 选中并开抽屉；双击 = 只开抽屉（避免双击误触编辑） |

#### A-3 冲突与提示

- 同房间重叠：在「单房间」模式由后端模板校验保证（`validation.rs:470`），前端拖拽阶段直接禁止落点；
- 跨房间同格：允许（不同房间），但格内 `+N` 上浮显示；
- 教师冲突：格内块加 amber 左边框，抽屉里显示冲突对象；
- 超出有效期：整块 45% 透明 + 角标 `超出有效期`，保存时统一阻止并列出；
- 单日覆盖：`全部房间` 模式不在 A 显示覆盖（A 是模板周）；工具栏若打开「显示覆盖徽标」，格右上角显示
  `覆盖×2` 小圆点，点击跳到 B 的对应日期。

#### A-4 状态

- 加载：表头先出，格内骨架；空：无课表行显示「该房间没有课表」+ 创建；
- 只读：块不可拖、抽屉只读、无保存条；
- 未保存：块左上角 4px accent 圆点 + 保存条；放弃 = 二次确认。

#### A-5 性能与取数

- 需要：区域 + 房间 + 课表 + 每课表 `weeklySlots`（8 房间 = 3+8 = 11 GET；需要显示「有无覆盖」时再 +8）；
- 渲染：只渲染聚合后的格内块（单房间 ≤42 块，全部房间 42×2 + 42 个 `+N` ≈ ≤150 节点），**不是** 336；
- 无需虚拟滚动；周切换不重新请求（模板与日期无关），仅当视图跨出课表有效期时改变空态。

---

### 3.B 方案 B · 资源时间轴

**心智模型**：Outlook 会议室排班。单日视图，行 = 房间（按区域分组，左粘性行头），横轴 = 时间，块绝对定位。

#### B-1 布局与尺寸

| 指标 | 1280×800 | 1440×900 |
| --- | --- | --- |
| 画布可用宽 | 976 | 1136 |
| 左侧行头宽 | 176px | 176px |
| 时间轴宽 | 800px | 960px |
| 时间范围 | 08:00–21:00（13h） | 08:00–21:00（13h） |
| 每小时宽 | 800/13 = **61.5px** | 960/13 = **73.8px** |
| 15 分钟吸附步长 | 15.4px | 18.5px |
| 30 分钟 | 30.8px | 36.9px |
| 时间表头 | 36px（每小时标签 + 半点刻度） | 36px |
| 区域组头高 | 28px（区域名 + 房间数 + 时区） | 28px |
| 房间行高 | 56px（8 房 = 448） | 56px |
| 总高（2 区域 8 房） | 36+2×28+8×56 = **540**（画布 500 → 纵向滚动 40，表头 sticky） | **540**（画布 600 → 完整可见） |
| 12 房（开发库压测） | 36+3×28+12×56 = **792** → 必须纵向滚动 | 同左 |
| 横向滚动 | 不需要（13h 刚好） | 不需要 |
| 粘性 | 时间表头 `sticky top-0`；行头 `sticky left-0`；左上角交叉格 `sticky top-0 left-0 z-20` | 同左 |

- 行头内容：房间名（12px 截断）+ 第二行 `区域名 · 容量`（10px）；右端 16px 课表切换 icon（多课表时）；
  无课表行头显示「无课表」+ `+`；只读时无 `+`。
- 行头点击 = 打开该房间的日视图详情（抽屉或 `/schedules/:id`）；行头 hover `bg-canvas/60`。
- 块：高度 = 行高 − 6 = 50px；宽度 = 时长 × 比例；最小可读宽 30min（1h 块 74px 可放 4–5 字课程名）；
  <30min 的块只显示颜色条 + tooltip（1280 下 15min 仅 15px，无法放文字）。
- 时间网格：整点 `border-line/70`，半点 `border-line/40` 虚线；非工作时间（08:00 前/21:00 后）不渲染。

#### B-2 核心交互

| 交互 | 规格 |
| --- | --- |
| 点击空白时间条 | 以该行房间 + 该 15min 起点新建：`pointerdown` 后不移动（<4px）松开 = 默认 90min 块；横向拖动 = 先选区间再松开，两种都弹 `BlockEditModal` 预填时间。**B 是具体日期 → 一律写单日覆盖，不写模板**（见 B-3） |
| 点击课块 | 选中 + 右侧抽屉；抽屉里显示来源 `模板 / 单日覆盖 / 停课`，并提供「查看所属课表 →」跳 `/schedules/:id` |
| 拖拽移动 | 横向改时间（15min 吸附，保留时长；clamp 08:00–21:00），纵向跨行改房间；跨区域 → 禁止（时区/权限）；拖动时显示 `09:30–11:00` 浮动时间标签 + 目标行高亮 + 冲突实时预览 |
| 跨房间拖动 | 目标房间 = 目标行；落库为「源课表该日期的 replace 覆盖，其中被移动时段的 `room_id=目标房间`」；若该日期已有覆盖 → 在覆盖里改；若目标房间同时间有课 → 阻止（rose） |
| 拖拽边缘改时长 | 左右 6px 手柄（cursor: ew-resize）；吸附 15min，最小 15min；拖动中实时显示 `开始–结束 · 90min`；越界/与自身其他段冲突 → 回弹 |
| 框选批量 | 不支持（C 的卖点） |
| 复制/粘贴 | 支持单块 Ctrl+C/V（同一天内不同房间/时间粘贴）；支持「复制整行 → 粘贴到另一房间同一天」（批量生成覆盖） |
| 撤销/重做 | 支持（草稿历史） |
| 键盘 | 方向键微调选中块 15min（上下换行=换房间）；Shift+方向键改时长；Delete 删除该块（若当天只剩活动块，提示「是否整天停课」→ cancel）；Esc 取消/关抽屉；Ctrl+S 保存 |
| 冲突预览 | 同房间同日期重叠 = 阻止；教师冲突 = 允许 + amber 提示；目标房间无课表 → 允许（写覆盖）但提示「该房间没有课表，将以单日覆盖方式排课」 |

#### B-3 单日覆盖语义（B 的核心难点，必须实现正确）

1. B 读：`roomOccurrences(roomId, {from: date, to: date})` 拿当天物化实例（已含例外与 room 覆盖，`source` 区分 `weekly/custom`）；
   同时对「主课表」拉 `exceptions(scheduleId, {from: date, to: date})` 判断当天是否已有 `cancel/replace`；
2. B 写（移动/改时长/新增/删除任意一块）：因为后端 `replace` 是**整天替换**（`README.md:350`），
   前端必须先把「当天生效的整套时段」算出来（模板当天时段，或已有 replace 的时段），只改目标块后整体
   `upsertException(scheduleId, date, { kind: 'replace', slots: 当天全部时段 })`；
3. 「删除当天最后一块」或抽屉里点「当天停课」→ `{kind:'cancel'}`（二次确认「当天全部课程停课」）；
4. 编辑前若发现该日期已有 `cancel`：提示「当天已停课，先恢复吗？」→ 转成 replace 或删除覆盖；
5. 超过 `今天 + SCHEDULE_HORIZON_DAYS`（默认 30 天）的日期：时间轴显示为空白，顶部提示「所选日期超出实例生成窗口（30 天），
   请先在详情页重建或调整窗口」；日期切换上限 = min(课表 end_date, 今天+horizon)；往前最多回看 7 天（更早实例可能未生成）；
6. 跨房间移动且源课表所属房间与目标房间不同：写**源课表**的当天 replace，槽位带 `room_id=目标房间`；
   该房间自己的课表若同日同时段有课，属于同房间冲突 → 阻止（需前端用目标房间的当天模板/实例做检测）。

#### B-4 冲突与提示

- 同房重叠：拖动/拉伸实时玫瑰提示并可回弹；保存前若有阻止级冲突 → 禁止保存并列出；
- 教师冲突：amber 边框 + 工具栏计数；
- 停课遮罩：`cancel` 当天整行画 rose 斜纹 + 「停课」徽标，块全部隐藏；抽屉里可「恢复当天」；
- 模板块 vs 覆盖块：B 中模板来源块用 `sky` 小角标「模板」，覆盖来源块用 `violet` 角标「覆盖」，停课无块；
- 多课表房间：行头课表切换器切换后，B 的块来源跟随该课表；切换器旁 `StateBadge tone=amber「多课表」`。

#### B-5 状态

- 加载：行头先出，时间轴区域骨架；空：无房间 → `EmptyState`；某行无课表 → 行内虚线 + 「创建课表/直接排课（单日覆盖）」；
- 只读：不可拖/不可建/抽屉只读；
- 未保存：块 accent 圆点 + 底部保存条；保存条额外显示「将写入 2 张课表的 N 天覆盖」。

#### B-6 性能与取数

- 每次日期切换：`roomOccurrences` × 可见房间数（8 → 8 GET）+ 每主课表 `exceptions` × 8；
  可只对选中日拉 exceptions（1 天，数据量小）；
- 缓存 `roomId|date`，前后一天预取（可选，用户连续翻页时受益）；
- 渲染 48–96 块，无虚拟滚动；12 房间 72 块也无需优化；
- 拖拽性能：只用 transform + 一个 `dropTarget` ref；时间标签用 `text` 更新（不要整块重渲染）。

---

### 3.C 方案 C · 全房间编排矩阵

**心智模型**：Excel 排课表。行 = 房间（按区域分组、左粘性），列 = 周一到周日 × 节次（横向滚动），
空格点击新建、鼠标框选批量铺课。适合「一次铺满所有房间」。

#### C-1 布局与尺寸

| 指标 | 1280×800 | 1440×900 |
| --- | --- | --- |
| 画布可用宽 | 976 | 1136 |
| 左侧房间列宽 | 168px | 168px |
| 可见列区宽 | 808px | 968px |
| 列结构 | 7 天 × 6 节 = **42 列** | 同左 |
| 列宽 | **88px** | 88px |
| 单天列组宽 | 616px | 616px |
| 可见天数 | 808/616 = **1.31 天**（约 9.2 列） | 968/616 = **1.57 天**（约 11 列） |
| 总内容宽 | 168 + 42×88 = **3864px** | 3864px |
| 横向滚动距离 | 2888px | 2696px |
| 行高（房间行） | 48px | 48px |
| 区域组头 | 28px | 28px |
| 表头两层 | 日 28px + 节次 36px = 64px | 64px |
| 总高（2 区域 8 房） | 64+2×28+8×48 = **512**（画布 500 → 纵向滚动 12） | **512**（画布 600 → 完整可见） |
| 粘性 | 左上角日/节表头 `sticky top-0`；房间列 `sticky left-0`；两层表头错 z-index | 同左 |
| 横向滚动提示 | 左/右边缘 `inset shadow` 渐隐 + 顶部「当前显示：周一 3–6 节 · 周二 1–2 节」位置标签 | 同左 |

**可读性妥协（必须写进交互）**

- 88px 列宽只能放约 6 个汉字（11px 字号）；课程名超过 6 字截断 + `title` 悬浮全名；
- 42 列一屏只能看到 ~1.5 天 → 必须提供以下导航，否则「一次铺满全周」会变成反复横向拖动：
  - 顶部「周一 周二 … 周日」快捷 chip，点击把该天滚到可视区；
  - `Shift + 滚轮` 横向滚动；`Ctrl + ←/→` 跳一天；`Home/End` 跳周一/周日；
  - 工具栏「列范围」筛选：`全天 / 上午(1-2) / 下午(3-4) / 晚上(5-6)`，默认全天，可选压缩视图；
  - 单区域筛选（8 → 4 行）时自动把行高提到 56px，提升可读性。
- 横向滚动条常驻（`overflow-x: scroll` 而非 auto，避免触控板用户找不到）。

#### C-2 核心交互

| 交互 | 规格 |
| --- | --- |
| 单击空格 | `pointerdown` → 若移动 <4px 且松开 = 单击：选中该格（accent 1px 内描边）+ 打开 `BlockEditModal` 预填（房间/星期/节次起止时间），Enter 保存生成 1 条草稿；若移动 ≥4px = 框选（见下一行），不弹窗 |
| 单击已占用格 | 选中该块 + 打开抽屉（与 A/B 一致）；Shift+单击扩选矩形 |
| 拖拽移动单块 | 拖到任意（房间 × 天 × 节）格；离散吸附；若源块跨多节，按「起点节次」移动并保留时长；目标格高亮 + 冲突预览；非法落点禁止 |
| 边缘改时长 | 不支持（离散矩阵）；时长改在抽屉/批量填充里做 |
| **框选批量** | `pointerdown` 在空格 → 拖出矩形 → `pointerup` 打开 `BatchFillModal`；选区 = 行范围（房间）+ 列范围（天×节）；跨区域不允许（先切区域）；跨天允许（如周一~周三同节次）；选区含已占用格时显示「将覆盖 3 个已有课程」警告 |
| 批量填充弹窗 | 字段：课程名*、教师、开始/结束时间（默认取选区首节 start / 末节 end，可改，必须晚于开始）、备注；`[x] 应用到选区内全部房间`（默认勾选）；提交前展示冲突预览（同房重叠/教师冲突条数）；提交 = 生成 N×M 条草稿 |
| 复制/粘贴 | 支持：选区 Ctrl+C（内部剪贴板，不碰系统剪贴板权限）→ 选中新锚点 Ctrl+V → 按相对偏移映射（行=房间偏移，列=天/节偏移），越界截断；粘贴覆盖已有课程前弹确认（列出数量） |
| 撤销/重做 | 支持（含框选批量整体作为一步） |
| 键盘 | 方向键移动选中格；Shift+方向键扩选；Enter 打开批量填充（多格）或抽屉（单块）；Delete 删除选区；Ctrl+C/V/Z/Shift+Z/S；Esc 取消选区 |
| 拖拽中的冲突预览 | 单块移动：与 A 相同的合法/非法判定；框选批量：提交前统一预检，冲突只警告不阻止（同房重叠则阻止，提示「选区内 P3 周一 1 节已有课，是否覆盖？」→ 确认后可覆盖 = 删除旧块 + 新建） |

#### C-3 冲突与提示

- 同房同时段：批量填充时若「应用全部房间」且某房间已有课 → 预览列出，用户选择「跳过已有 / 覆盖」；
- 教师冲突：批量填充后若同教师跨房间同时间 → 警告汇总（「3 处教师时间冲突」），不阻止；
- 超出有效期：批量填充的 weekday 都落在模板里，无「日期超出」问题；但若主课表有效期已过 → 行置灰并阻止新建；
- 单日覆盖：C 默认模板周，显示覆盖徽标（格右上角小点），点击切到 B 当天；
- 视觉：选区 `bg-accent-soft/60 ring-1 ring-accent`；已占用格中的块保持原色；批量填充后的新块短暂 `ring-2 ring-emerald-400` 1.2s 高亮。

#### C-4 状态

- 加载：房间列先出，列头先出，单元格骨架；
- 空：有房间无课表 → 行内虚线 + 批量创建入口；有课表无课程 → 空格提示「框选多格批量填充」；
- 只读：禁止框选/拖拽/批量，单元格 hover 只显示 tooltip；抽屉只读；
- 未保存：新块 accent 圆点；保存条显示「将写入 N 张课表 · M 条时段」；
- 保存中：按课表串行进度（`3/8 张课表已保存`），已完成课表的块变为实心（不再是草稿态）。

#### C-5 性能与取数

- 数据同 A：3 + 8 个 GET；框选批量保存 = 受影响课表各 1 次 PUT（串行），有批量接口则 1 次；
- 渲染：336 空 cell + 336 块 + 42 列头 ≈ 700–900 组件；Vue 3 可承受，但**必须**：
  - 用固定高度/宽度的 CSS（`table-fixed` 或 `grid-template-columns` 常量），避免单元格内容撑开导致重排；
  - 空 cell 用 CSS 背景网格绘制（`repeating-linear-gradient`）而不是 336 个 div 边框；
  - 块组件 `v-memo="[block.key, block.dirty, selectedKey]"`；
  - 横向滚动时不触发响应式重算（滚动位置用原生 scroll，不 v-model 绑定 scrollLeft）。
- 12 房间 × 7 天 × 6 节 = 504 块，仍在可接受范围；若房间数 >20，建议按区域折叠为「区域汇总行」，
  而不是引入虚拟滚动（虚拟滚动会破坏框选/拖拽的坐标计算，成本远高于收益）。

---

### 3.D 三方案对比

| 维度 | A 周网格课表 | B 资源时间轴 | C 全房间编排矩阵 |
| --- | --- | --- | --- |
| 心智模型 | 教务系统课表 | Outlook 会议室排班 | Excel 排课表 |
| 时间维度 | 模板周（weekday） | 具体日期（真实课表） | 模板周（weekday） |
| 行 × 列 | 节次 × 周一~周日 | 房间（按区域分组）× 时间 | 房间（按区域分组）× 周×节次 |
| 最适合场景 | **日常查看**：某房间/某区域这周怎么排；单房间精细调整；教师/课程时间分布 | **调课/冲突处理**：某一天所有房间占用、临时换房、停课、拉伸时长 | **批量铺设**：开学初/换季时一次把 8 个房间 × 5 天铺满，或在矩阵里改一周 |
| 8 房间可读性 | 全部房间需聚合成 `+N`；单房间/区域最佳 | 8 行刚好一屏（1440）；12 房需纵向滚动 | 横向一屏仅 ~1.5 天；必须配合快捷导航 |
| 横向滚动 | 无 | 无（13h 恰好） | **必须**（2888px） |
| 纵向滚动 | 无 | 1440 无 / 1280 轻微 / 12 房需要 | 1440 无 / 1280 轻微 |
| 拖拽改时间 | 吸附节次 | 吸附 15min，可拉伸 | 吸附节次（离散） |
| 跨房间拖动 | 通过抽屉改房间（覆盖） | **原生支持**（跨行） | 支持（拖到另一行） |
| 框选批量 | 无 | 无 | **核心能力** |
| 复制整周 | 支持（单房间→另一房间） | 单日行复制 | 矩形复制粘贴 |
| 撤销/重做 | 有 | 有 | 有（批量算一步） |
| 写库语义 | 周模板全量替换 | 单日覆盖 upsert | 周模板全量替换（可跨课表批量） |
| 实现成本（前端） | 2.5d（只读 1.5d + 编辑 1d） | 3d | 2.5d |
| 主要风险 | 全部房间密度不可读、节次模板假设 | 单日覆盖「整天替换」语义、跨房间落库契约、horizon 30 天 | 横向滚动体验、框选与拖拽并存的手势冲突 |
| 建议上线顺序 | **P1（默认视图）** | P3 | P4（开学期前上线） |

**推荐组合**（与 Lead 倾向一致，但补充优先级）：

- `/schedules` = 排课工作台，顶部 `FilterTabs` 三段切换：`周网格 | 时间轴 | 编排矩阵`；
- 默认视图 = **A + 区域筛选**（8 房间时默认选中第一个区域，2 区域时 A 也可「全部房间」聚合模式）；
- 把「某一天要调课」「冲突处理」引导到 B（A 中点击覆盖徽标 → 自动切 B 并定位日期）；
- 把「批量铺课」引导到 C（空状态/工具栏「批量铺课」按钮 → 切 C 并聚焦第一个空区域）；
- 三视图共享同一份 store：筛选、草稿、冲突、选中块、保存条。

---

## 4. 推荐 IA 与组件树

### 4.1 路由方案（3 选 1）

| 方案 | 做法 | 优点 | 缺点 | 结论 |
| --- | --- | --- | --- | --- |
| ① 改造 `/schedules` | `name: schedules` 指向 `ScheduleBoardView`；旧列表挪 `/schedules/list` | 侧栏/搜索/权限/`schedule-detail` 全部不动；用户点「课表」直接进工作台；无新增导航项 | 现有外部链接/书签的「课表列表」会变成工作台（可接受，符合用户诉求） | **推荐** |
| ② 新增 `/timetable` | 侧栏加「排课工作台」，`/schedules` 仍为列表 | 零破坏 | 两个课表入口，用户要学两次；导航拥挤；`filterTabs` 价值被稀释 | 不推荐 |
| ③ `/schedules` 内 tab 切「列表/工作台」 | 同一路由 query 切换 | 只改一个文件 | 列表与画布差异太大，列表分页/筛选与画布筛选两套状态，体验割裂 | 备选 |

**推荐 ①** 的具体路由：

```ts
// routes.ts（示意，实际由 Lead 改）
{ path: 'schedules', name: 'schedules', component: ScheduleBoardView, meta: { title: '课表', permission: scheduleRead } },
{ path: 'schedules/list', name: 'schedule-list', component: SchedulesListView, meta: { title: '课表列表', permission: scheduleRead, activeName: 'schedules' } },
{ path: 'schedules/:id', name: 'schedule-detail', component: ScheduleDetailView, meta: { title: '课表详情', permission: scheduleRead, activeName: 'schedules' } }, // 不变
```

- 工作台 URL 状态（便于分享/刷新恢复，用 `router.replace` 避免历史爆炸）：
  `?view=week|timeline|matrix & week=2026-11-02 & date=2026-11-03 & area=<id> & room=<id> & schedule=<id>`
- 兼容：`CommandPalette` 的 `nav-schedules` 仍落 `name: schedules` → 工作台（`CommandPalette.vue:79-85`）；
  搜索具体课表仍落 `schedule-detail`（`:213-220`），无需改。
- 旧列表页保留：工具栏右上「课表列表」按钮 → `/schedules/list`；空状态与详情页返回链接同步改（`ScheduleDetailView.vue:234-240`
  的「返回课表列表」→ 回工作台；工作台里保留「列表」入口）。

### 4.2 页面结构（wireframe）

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ 课表 / 排课工作台                                              [刷新] [列表] [+ 新建课表] │  PageHeader
│ 一个界面编排全部房间：模板周看排课，时间轴调某天，矩阵批量铺课。                    │
├──────────────────────────────────────────────────────────────────────────────┤
│ [周网格][时间轴][编排矩阵]   ‹ 今天 ›  2026-11-02 ~ 11-08  [区域▾][房间▾] [只看冲突] │  BoardToolbar 40px
│   模板周 · 2026 秋季（11-02 ~ 01-31）               ⚠ 2 阻止 · 1 警告   [密度▾]     │
├──────────────────────────────────────────────────────────────────────────────┤
│  时间   周一11-02   周二11-03   周三11-04   周四11-05   周五11-06   周六11-07  周日 │  表头(sticky)
│ 08:30 ┌──────────┬──────────┬──────────┬──────────┬──────────┬────────┬───────┐ │
│ 10:00 │ 高数·P3   │ +2       │ 英语·P1   │          │          │        │       │ │
│ 10:10 │ …         │          │          │          │          │        │       │ │
│ …     │           │          │          │          │          │        │       │ │
├──────────────────────────────────────────────────────────────────────────────┤
│ 〔6 项未保存〕 [撤销][重做] [放弃]                          [保存到 2 张课表 →]     │  BoardSaveBar
└──────────────────────────────────────────────────────────────────────────────┘
                                        ┌───────────────────────────┐
                                        │ 课程详情            [×]   │  BlockDetailDrawer 380px
                                        │ P3 房间 · 高数 · 王老师    │  （fixed right，覆盖在画布上）
                                        │ 周一 08:30–10:00（模板）   │
                                        │ [房间▾][开始][结束]        │
                                        │ 备注 …                     │
                                        │ [删除]        [保存到草稿] │
                                        └───────────────────────────┘
```

B 视图 wireframe（行头粘性）：

```
│ 房间/时间 │ 08:00  09:00  10:00  11:00  12:00  13:00 … 21:00 │
│ ▼ P 区域 · 4 房 · Asia/Shanghai                                │
│ P3 房间   │      ██████ 高数 P3   ▓▓▓ 英语      ██ 物理        │
│ P1 房间   │ ███ 化学                  █████ 高数              │
│ ▼ P 区域 · 4 房                                               │
│ …                                                            │
```

C 视图 wireframe（横向滚动 + 粘性）：

```
│ 房间        │ 周一 1 │ 2 │ 3 │ 4 │ 5 │ 6 │ 周二 1 │ 2 │ …（42 列）│
│ ▼ P 区域    │        │   │   │   │   │   │        │   │          │
│ P3 房间     │        │   │ 高数  │   │   │   │        │   │          │
│ P1 房间     │ 化学    │   │       │   │   │   │        │   │          │
│ …           │        │   │       │   │   │   │        │   │          │
│            ◀ 横向滚动条 ▶    [周一][周二][周三]… 快捷定位              │
```

### 4.3 组件拆分（文件名 + props/emits 草案）

| 组件 | props | emits | 说明 |
| --- | --- | --- | --- |
| `views/ScheduleBoardView.vue` | — | — | 页面编排：取数、视图分发、路由 query 同步、工具栏/画布/抽屉/保存条组装 |
| `BoardToolbar.vue` | `view: ViewMode`、`anchor: string`、`weekStart: string`、`weekEnd: string`、`areas: Area[]`、`rooms: Room[]`、`areaId: string`、`roomId: string`、`onlyConflict: boolean`、`conflicts: {blocking:number; warning:number}`、`density: 'compact'\|'standard'`、`readonly: boolean`、`loading: boolean` | `update:view`、`update:areaId`、`update:roomId`、`update:onlyConflict`、`update:density`、`navigate: [direction:-1\|0\|1]`、`today`、`refresh`、`openList`、`createSchedules`、`openBlock: [key:string]` | `FilterTabs` + 日期导航 + 筛选 + 冲突计数 |
| `BoardCanvasShell.vue` | `loading`、`error`、`emptyKind: 'no-room'\|'no-schedule'\|'filtered'\|''`、`readonly` | `retry`、`clearFilter`、`createRoom` | 统一滚动容器 + 粘性 + 空/错/骨架；内部 `<slot>` 放三种画布 |
| `WeekGridView.vue` | `blocks: BoardBlock[]`、`rooms: BoardRoom[]`、`mode: 'all'\|'area'\|'room'`、`anchorWeek: string`、`selectedKey: string`、`readonly`、`density`、`conflicts: Map<string, Conflict>`、`showOverrideBadge: boolean` | `select: [key]`、`createAt: [dayOfWeek, periodIndex]`、`moveBlock: [key, dayOfWeek, periodIndex]`、`resizeBlock`（A 不用）、`openDay: [date]` | 7 列 × 6 行；全部模式聚合 `+N`；表头/时间列粘性 |
| `WeekGridBlock.vue` | `block: BoardBlock`、`conflict: Conflict \| null`、`selected: boolean`、`readonly`、`density` | `select`、`dragstart: [key, PointerEvent]` | 单块视觉；用 `ScheduleBlockChip` 内部 |
| `ResourceTimelineView.vue` | `date: string`、`rooms: BoardRoom[]`、`blocks: BoardBlock[]`、`exceptions: ScheduleException[]`、`timezone: string`、`selectedKey`、`readonly`、`conflicts`、`dragState` | `select`、`createAt: [roomId, start, end]`、`moveBlock`、`resizeBlock`、`moveToRoom`、`requestCancel: [scheduleId, date]`、`dateChange` | 单日；行头粘性；时间轴 08:00–21:00；块绝对定位 |
| `TimelineBlock.vue` | `block`、`leftPct`、`widthPct`、`rowHeight`、`conflict`、`selected`、`readonly`、`dragging` | `select`、`pointerdown: [kind:'move'\|'resize-start'\|'resize-end', ev]` | 50px 高；<30min 只色条；edge handle 6px |
| `AllRoomMatrixView.vue` | `blocks`、`rooms`、`weekStart`、`selection: MatrixSelection`、`selectedKey`、`readonly`、`conflicts`、`periodTemplate` | `selectCell: [roomId, day, period]`、`beginBoxSelect: [PointerEvent]`、`moveBlock`、`openBatchFill: [selection]`、`pasteAt: [anchor]`、`scrollToDay: [day]` | 42 列；`useMatrixSelection`；横向滚动 |
| `MatrixCell.vue` | `roomId`、`day`、`period`、`block?: BoardBlock`、`selected`、`inSelection`、`readonly` | `activate`、`pointerdown` | 极简（背景格子 + 命中的块）；`v-memo` |
| `ScheduleBlockChip.vue` | `title`、`roomName?`、`teacher?`、`start`、`end`、`tone: 'normal'\|'conflict'\|'warning'\|'muted'`、`dirty`、`readonly`、`compact` | `click` | 三视图共用视觉（左边框色/角标/时间） |
| `BlockDetailDrawer.vue` | `open`、`block: BoardBlock \| null`、`schedule: Schedule \| null`、`rooms: Room[]`、`readonly`、`dirty`、`saving`、`conflict: Conflict \| null` | `close`、`update: [patch: Partial<SlotDraft>]`、`delete`、`openSchedule: [scheduleId]`、`save` | 380px；`fixed right-0 top-14 bottom-7`；Esc 关闭；焦点循环 |
| `BlockEditModal.vue` | `open`、`mode: 'create'\|'edit'`、`prefill: { roomId?, dayOfWeek?, start?, end?, date? }`、`rooms`、`saving`、`readonly` | `submit: [BoardEditDraft]`、`close` | 新建/编辑表单；课程名/教师/备注/房间/星期/起止；A/C 用 weekday，B 用 date（隐藏星期） |
| `BatchFillModal.vue` | `open`、`selection: MatrixSelection`、`rooms`、`existingCount`、`previewConflicts`、`saving` | `submit: [BatchFillPayload]`、`close` | C 框选；应用全部房间 checkbox；冲突预览 |
| `CreateSchedulesModal.vue` | `open`、`roomsWithoutSchedule: BoardRoom[]`、`defaultRange: {start; end}`、`saving` | `submit: [roomIds, range, namePattern]`、`close` | 批量初始化课表；名称模板 `{房间名} 课表` |
| `ConflictSummary.vue` | `blocking`、`warning`、`items: Conflict[]` | `locate: [blockKey]`、`close` | 工具栏计数 + 弹层列表 |
| `BoardSaveBar.vue` | `dirtyCount`、`dirtySchedules`、`saving`、`progress: {done,total}`、`canUndo`、`canRedo`、`readonly` | `undo`、`redo`、`discard`、`save` | 毛玻璃底条；`position: sticky bottom-0` |
| `BoardLegend.vue` | `showOverride`、`showTeacher` | — | 颜色/冲突/覆盖/停课图例 |
| `BoardEmptyState.vue` | `kind`、`roomCount`、`scheduleCount` | `createRoom`、`createSchedules`、`clearFilter` | 三种空态 |

**Composables**

| 文件 | 职责 | 关键 API |
| --- | --- | --- |
| `useBoardData.ts` | 取区域/房间/课表/模板/覆盖/实例；缓存与失效；并发池；逐行状态 | `loadBase()`、`loadTemplates(scheduleIds)`、`loadExceptions(range)`、`loadOccurrences(date)`、`invalidate(kind, id)`、`rows`、`rowState` |
| `useBoardDraft.ts` | 两类草稿（`templateDraft: Map<scheduleId, DraftSlot[]>`、`dayDraft: Map<scheduleId\|date, DraftException>`）、脏标记、历史快照、保存合并、撤销重做 | `apply(edit: BoardEdit)`、`undo()`、`redo()`、`discard()`、`save()`、`dirtyCount`、`canUndo/canRedo` |
| `useBoardConflicts.ts` | 由块/草稿构建冲突索引；`expandDraftToBlocks()`、`conflictsByBlock`、`counts` | `rebuild(blocks)`、`preview(candidate)`、`conflictsFor(key)` |
| `useBlockDrag.ts` | 通用 Pointer Events 拖拽（A/B/C 共用，模式不同） | `start(e, block, mode)`、`dragState`、`dropTarget`、`cancel()`、回调 `onPreview/onDrop` |
| `useMatrixSelection.ts` | C 的矩形选区、Shift 扩展、剪贴板、粘贴映射 | `begin(roomId, day, period)`、`extend(...)`、`selection`、`copy()`、`pasteAt(...)`、`clear()` |
| `useBoardKeyboard.ts` | 全局快捷键分发（输入框内不拦截） | `onKeydown(e)`、`install()/dispose()` |

### 4.4 Pinia store

位置：`frontend/src/features/scheduleBoard/boardStore.ts`（**新建**，不动 `stores/auth|shell|ui|toast.ts`，
符合 `CONVENTIONS.md:85` 的限制）。建议 setup store：

```ts
export const useBoardStore = defineStore('scheduleBoard', () => {
  // 视图与筛选（同步到 URL query，由页面 watch）
  const view = ref<'week'|'timeline'|'matrix'>('week')
  const anchorDate = ref(today())          // B 的具体日期
  const weekStart = ref(mondayOf(today())) // A/C 的模板周锚点（仅用于显示日期区间）
  const areaId = ref('')
  const roomId = ref('')
  const onlyConflict = ref(false)
  const density = ref<'compact'|'standard'>('standard')
  const readonly = computed(() => !auth.can(PERMISSIONS.scheduleManage))

  // 数据（readonly 快照 + 行状态）
  const areas = ref<Area[]>([])
  const rooms = ref<BoardRoom[]>([])       // { room, area, primarySchedule, scheduleCount, rowState }
  const templates = ref<Map<string, WeeklySlot[]>>()
  const exceptions = ref<Map<string, ScheduleException[]>>()
  const occurrences = ref<Map<string, Occurrence[]>>()
  const loading/error 等

  // 草稿与历史（只读用户为空）。两类草稿分开，保存时分别走不同接口
  const templateDraft = ref<Map<string /*scheduleId*/, DraftSlot[]>>()        // A/C：周模板
  const dayDraft = ref<Map<string /*scheduleId|date*/, DraftException>>()     // B：单日覆盖
  const history = ref<HistorySnapshot[]>()
  const redoStack = ref<HistorySnapshot[]>()

  // 选中/拖拽/冲突
  const selectedKey = ref('')
  const dragState = ref<DragState|null>()
  const conflictIndex = ref<Map<string, Conflict>>()
  const conflictCounts = computed(() => ...)

  // actions
  async function loadBase()
  async function loadVisibleData()          // 按 view 决定拉模板/覆盖/实例（并发池）
  function setView(v) / navigateDay(n) / navigateWeek(n) / setArea(id) / setRoom(id)
  function createSlot(prefill) / updateSlot(edit) / moveSlot(edit) / deleteSlot(key) / batchFill(payload)
  function copySelection() / pasteAt(anchor)
  function undo() / redo() / discard()
  async function saveAll()                  // 先串行 PUT weekly-slots（A/C 脏课表），再逐天 PUT exceptions（B 脏日期）
  function rebuildConflicts()
  function locateBlock(key)                 // 切视图 + 滚动到块 + 选中
})
```

- **草稿不写回服务端快照**：`templates/exceptions/occurrences` 永远是最近一次服务端快照；渲染时
  `blocks = expand(templateDraft ?? templates, dayDraft ?? exceptions)`；撤销/放弃只需丢弃草稿；
- `saveAll()` 的保存顺序与进度文案：
  1. 有 `templateDraft` 的课表逐张 `PUT /schedules/:id/weekly-slots`（全量替换，成功即后端重建未来实例）；
  2. 有 `dayDraft` 的日期逐天 `PUT /schedules/:id/exceptions/:date`（replace/cancel）；
  3. 两类保存都串行，进度条区分「模板 2/3 · 覆盖 4/5」，失败的保持脏并列出；
  4. 全部成功后失效 templates/exceptions/occurrences 缓存、重建冲突、清空历史。
- store 内不做路由跳转（页面负责 `router.replace` 同步 query），与现有 `stores/overview.ts` 的「store 取数、页面编排」一致；
- 每个画布组件只通过 props 拿块、通过 emits 抛操作，**不直接调 api**（符合 `CONVENTIONS.md:72`）。

### 4.5 与 `/schedules/:id`、房间/区域页的关系

| 位置 | 改动 | 理由 |
| --- | --- | --- |
| `/schedules` | 换成 `ScheduleBoardView`；顶部/空状态保留「课表列表」入口 | 工作台成为课表主入口 |
| `/schedules/list` | 现 `SchedulesView` 原样迁移（改 View 名/路由） | 保留分页列表与「新建课表」表单，零功能损失 |
| `/schedules/:id` | **保留**，作为「单房间精细维护」：基本信息 + 周模板表格 + 单日覆盖 + 实例 | 兼容已有习惯；画布不适合做批量/审计类操作 |
| 详情页头部 | 加「在排课工作台打开」按钮：`/schedules?view=week&room=<room_id>&schedule=<id>` | 双向入口 |
| `RoomDetailView` / `RoomSchedulesPanel` | 课表行加「在工作台排课」链接（同一 URL） | 从房间现场跳到画布 |
| `AreaDetailView` | 课表区块加「在排课工作台查看本区域」→ `?area=<id>` | 区域级入口 |
| `CommandPalette` | 不必须改；`nav-schedules` 落工作台即可 | 见 §4.1 |
| `ScheduleDetailView` 的保存逻辑 | 不动（仍全量替换 + 重建）；工作台复用同一 API | 避免双写逻辑 |

**入口文案**：`课表` → 工作台标题「课表 · 排课工作台」，副标题「一个界面编排全部房间：模板周看排课、时间轴调某天、矩阵批量铺课」。

---

## 5. 实现阶段建议（每步可独立验收）

> 前置：Lead 与后端任务（task-1）先定「跨房间移动的落库语义」与「主课表规则」；前端阶段 1/2 不依赖它们。

### 阶段 1（2.5d）：数据层 + A 周网格只读 + 路由切换

**交付**：`/schedules` 工作台壳 + A 视图只读；筛选（区域/房间/搜索/只看冲突）；冲突计数；空/错/加载态；
URL query 状态；旧列表迁到 `/schedules/list`；无课表房间行 + 「批量创建课表」入口（先串行 `createSchedule`）。

**验收**：
- 8 房间、2 区域数据下，A 视图 1 次进入能完整渲染；单房间/区域模式块信息完整；
- 全部房间模式出现 `+N` 聚合而不是溢出/换行；
- 切区域/房间/搜索 300ms 内出结果；无课表行有明确引导；
- `vue-tsc --noEmit` 通过；无新依赖。
**依赖后端**：现有 `listAreas/listRooms/listSchedules/weeklySlots`；不需要新接口（N 请求可接受）。

### 阶段 2（3d）：编辑闭环（A 可写）

**交付**：`BlockEditModal` 新建/编辑、`BlockDetailDrawer`、单块删除、拖拽改星期/节次、改房间（覆盖）、
草稿/脏标记/撤销/重做/放弃、`BoardSaveBar`、`beforeunload` + 路由离开确认、只读降级、
保存串行 + 部分失败重试；复制单块与「复制整周」。

**验收**：
- 新建/移动/删除→保存后刷新页面结果一致；后端重叠 400 能定位到具体块并保留草稿；
- 撤销/重做 50 步不崩；放弃二次确认；
- 只读账号看不到任何写入口，直接改 URL 也不会触发写请求；
- 保存 2 张课表时逐张显示进度，失败的那张仍为脏。
**依赖后端**：`replaceWeeklySlots`、`createSchedule`（批量建课表可选，先串行）；无新接口也能交付。

### 阶段 3（3d）：B 资源时间轴（真实日期 + 单日覆盖）

**交付**：B 视图（日导航、区域分组、粘性行头/表头、绝对定位块）、跨房间/改时间拖拽、边缘改时长、
冲突实时预览、当天整套 replace 覆盖写入、停课/恢复、超 horizon 提示、模板/覆盖来源角标。

**验收**：
- 拖动一节课 → 只影响当天，`exceptions` 中生成整天 replace，刷新后位置保持；
- 同房同时段阻止落点；教师冲突 amber 警告；跨区域拖动被禁止；
- 日期 > 今天+30 天时给出明确提示且不可编辑；
- 已停课当天再编辑 → 引导恢复为 replace，不产生两个覆盖。
**依赖后端**：现有 `roomOccurrences` + `exceptions/upsertException/deleteException`；
**跨房间落库语义需要后端确认**（见 §6-Q6），否则阶段 3 只做同房间移动/改时间。

### 阶段 4（2.5d）：C 全房间编排矩阵 + 批量

**交付**：C 视图（42 列横向滚动、粘性房间列/两层表头、快捷日定位）、框选与 Shift 扩选、
`BatchFillModal`（应用全部房间、冲突预览、覆盖确认）、矩形复制粘贴、批量保存进度、`v-memo` 性能优化。

**验收**：
- 8 房间 × 周一~周五 × 1-2 节 一次框选铺满，保存后 8 张课表全部正确；
- 横向滚动 60fps 无卡顿、表头/房间列不跑偏；框选与拖拽手势不打架（空格起框选、块上下起拖拽）；
- 覆盖已有课程前必须确认，取消后无副作用；
- 批量保存部分失败时仅失败课表保留脏标记。
**依赖后端**：`replaceWeeklySlots`（N 次串行可接受）；有 `batchReplaceWeeklySlots` 更佳；
区块/单块复制粘贴是纯前端状态，无接口依赖。

### 收尾（1.5d）

- 三视图回归 + `vue-tsc --noEmit` + `oxlint/eslint` + `oxfmt`；
- Playwright 冒烟（如可跑）：`/schedules` 三视图切换、单块新建→保存→刷新、只读账号；
- 1440×900 与 1280×800 截图对齐 `docs/screenshots/` 占位；
- 同步更新 `frontend/docs/CONVENTIONS.md` 的「页面与文件归属」表（新增 `features/scheduleBoard/` 与路由说明）。

**总估**：12–13 人天前端（不含后端接口开发与联调等待）。并行策略：阶段 1 完成后，
B（阶段 3）与 C（阶段 4）可由两人分别开发（共享阶段 2 的草稿/拖拽/冲突 composables），互为独立验收。

---

## 6. 待用户确认的问题（≤8，每条给默认建议）

| # | 问题 | 默认建议 |
| --- | --- | --- |
| Q1 | 三视图中哪个作为默认落地/默认打开？ | **A 周网格**默认，B/C 同批上线；默认带「区域」筛选（8 房间时默认选第一个区域，可选全部房间聚合） |
| Q2 | A/C 的「周」是**模板周**（改一次影响整个有效期）还是「本周」（只改这一周、写单日覆盖）？ | **模板周**，UI 明示「模板」；「只改这一周」引导到 B（写单日覆盖） |
| Q3 | 节次从哪来（后端无节次实体）？是否要做成可配置（区域级/全局）？ | 先**前端固定 6 节常量**（`08:30–10:00 … 18:30–20:00`），块内显示真实时间；节次配置放二期 |
| Q4 | 入口与路由：改造 `/schedules`（列表挪 `/schedules/list`）还是新增 `/timetable`？ | **改造 `/schedules`**，不新增侧边栏项，保留旧列表路由 |
| Q5 | 一房多课表如何呈现？是否要求后端加「每房唯一启用课表」约束？ | 前端按「有效期内 + 启用 + updated_at 最新」选主课表，行头可切换；是否加约束由后端定（前端两种都能适配） |
| Q6 | 跨房间拖动（B）如何落库：写**源课表**当天的 replace 覆盖 + `room_id` 覆盖，还是写到**目标房间**的课表？ | 先做「同房间移动/改时间」；跨房间拖动**等后端路径确定后再开**（依赖 task-1 的选型），默认按「源课表 override」实现 |
| Q7 | 冲突策略：同房重叠阻止；教师同名仅警告；超有效期阻止；覆盖/停课提示不阻止——是否接受？ | 接受上述默认；教师冲突明确标注「按姓名精确匹配，可能误报」 |
| Q8 | C 框选批量范围：允许「同房间跨天」和「同天跨房间」，不允许跨区域（时区/权限）——是否接受？ | 接受；跨区域先切区域再做；「应用全部房间」默认勾选 |

---

## 附：实测环境与数据

- 后端 `127.0.0.1:8080` 运行中（`/healthz` 200），admin 账号只读探测：
  `GET /admin/areas` → 12 条，`GET /admin/rooms` → 12 条，`GET /admin/schedules` → 1 条（名称 `test`，周模板为空）。
- 开发库存在同名区域/房间（多个 `agent config test area/room`），因此工作台的排序与分组键必须是
  `area_id + name + code`，不能用 name 作 key；行/块 key 用 `scheduleId + slotId` 或 `scheduleId + date + exceptionSlotId`。
- 题目给定的目标规模：**2 区域 / 8 房间**（`frontend/docs/screenshots/*.png` 中顶栏也显示 8 个房间）。
- 现有截图（视觉基线）：`frontend/docs/screenshots/schedules.png`（空列表卡片）、`room-detail.png`（双列信息卡+右侧栏）、
  `area-detail.png`（分块事件流）。工作台应沿用 `rounded-xl` 卡片、`border-line`、10.5–13.5px 字号与 accent 蓝，
  画布本身允许更低的边框对比度（网格线 `line/40`）。
