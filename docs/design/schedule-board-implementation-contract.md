# 排课工作台实现契约（v1）

> 状态：**v1 已实现**（2026-11-01）；**v1.5 单日调课**见 §5.1、**v1.6 撤销/重做**见 §5.2、
> **v1.7 C 矩阵框选批量**见 §5.3、**v1.8 C 矩阵复制/粘贴**见 §5.4。
> 本文件是「课表重设计」落地实现的唯一契约，后端 DTO、前端类型、组件 props/emits 都以此为准。
> 设计背景见 `docs/schedule-board-redesign.md`；不改 `docs/schedule-design.md` 的既有契约。
> 实现位置：后端 `crates/api/src/api/admin/schedule_board.rs` + `crates/db/src/schedule.rs`；
> 前端 `frontend/src/views/ScheduleBoardView.vue` + `frontend/src/features/scheduleBoard/`。

## 1. 范围（v1）

- `/schedules` 从「课表列表」改为「排课工作台」，旧列表挪到 `/schedules/list`（路由已由 Lead 改好）。
- 三视图共享真实数据：A 周网格（默认）、B 资源时间轴、C 全房间编排矩阵。
- **编辑 = 改周模板**：新建/编辑/删除/拖拽课块，最终都落到某张课表的 `PUT /schedules/{id}/weekly-slots`
  （全量替换）+ `POST /schedules/{id}/rebuild`，刷新后生效。**v1 不做单日调课/撤销**；单日覆盖只读展示。
  **v1.5 起 B 时间轴支持单日调整**（默认作用域「本次」，写 `schedule_exceptions`），A/C 仍固定模板语义；见 §5.1。
- 拖拽在 A（改星期/节次）与 B/C（跨房间、跨天、改时间）都支持；B 改时长（拉伸）v1 不做。
- 权限：`schedule:read` 查看；`schedule:manage` 才显示写入口/允许拖拽。

## 2. 后端接口

### 2.1 `GET /api/v1/admin/schedule-board`

权限 `schedule:read`。Query：`from`（YYYY-MM-DD，必填）、`to`（必填，`to >= from`，跨度 ≤ 62 天）、
`area_id`（可选 UUID）。

响应（字段一律 snake_case，缺省值给 `null`/`[]`，不要省略键）：

```json
{
  "from": "2026-11-02",
  "to": "2026-11-08",
  "areas": [
    { "id": "uuid", "code": "p3", "name": "P3 区域", "timezone": "Asia/Shanghai" }
  ],
  "rooms": [
    {
      "id": "uuid", "area_id": "uuid", "area_name": "P3 区域", "code": "p3-room",
      "name": "P3 房间", "capacity": 50, "device_slot": "room-501",
      "is_active": true, "timezone": "Asia/Shanghai",
      "device": { "serial": "2e4d…", "status": "active", "online": true }
    }
  ],
  "schedules": [
    {
      "id": "uuid", "room_id": "uuid", "name": "默认课表",
      "start_date": "2026-09-01", "end_date": "2027-01-31",
      "is_active": true, "timezone": "Asia/Shanghai"
    }
  ],
  "weekly_slots": [
    {
      "id": "uuid", "schedule_id": "uuid", "day_of_week": 1,
      "start_time": "08:00", "end_time": "09:40",
      "course_name": "高等数学", "teacher": "张伟",
      "room_id": null, "note": ""
    }
  ],
  "exceptions": [
    {
      "id": "uuid", "schedule_id": "uuid", "exception_date": "2026-11-03",
      "kind": "cancel", "note": "",
      "slots": [
        {
          "id": "uuid", "start_time": "08:00", "end_time": "09:40",
          "course_name": "高等数学", "teacher": "张伟",
          "room_id": null, "note": ""
        }
      ]
    }
  ]
}
```

实现口径（必须一致）：

- `areas`：全部区域（含停用），按 `name` 排序；仅用于前端筛选下拉。`area_id` 只过滤 rooms，不裁剪 areas。
- `rooms`：`is_active = true` 的房间；有 `area_id` 时只返回该区域房间；按 `area.name, room.code` 排序。
- `schedules`：`is_active = true` 且属于返回房间的课表（不限日期，前端自己夹日期）。
- `weekly_slots`：属于上述课表的全部周模板，按 `day_of_week, start_time` 排序。
- `exceptions`：属于上述课表、且 `exception_date BETWEEN from AND to` 的覆盖；`slots` 按 `start_time` 排序。
- `device`：可为 `null`（房间未绑定或无 enrollment）；`online` 取自设备在线状态，取不到给 `null`。
- `from/to` 非法 → 400；无权限 → 403；错误格式沿用现有 `AppError`。
- 性能：禁止 N+1（按 schedule_id 集合批量查 `weekly_slots`/`exceptions`，按 room 集合批量查 device）。

### 2.2 `POST /api/v1/admin/schedules/ensure`

权限 `schedule:manage`。请求：

```json
{ "room_ids": ["uuid"], "start_date": "2026-09-01", "end_date": "2027-01-31", "name": "默认课表" }
```

- `name` 缺省 `默认课表`；`end_date >= start_date`；`room_ids` 去重、上限 100、必须存在且启用。
- 每个房间：若已存在 `is_active = true` 且与 `[start_date, end_date]` 有交集的课表 → 归入 `existing`
  （取 `start_date` 最早的一份）；否则新建一份 → 归入 `created`。
- 响应：`{ "created": [Schedule…], "existing": [Schedule…] }`，`Schedule` 沿用现有 `GET /schedules` 的
  响应字段（`id/room_id/room_name/name/start_date/end_date/timezone/is_active/created_by/created_at/updated_at`）。
- 幂等：同参数重复调用不产生重复课表。
- 审计：action `schedule.ensure`，detail 记录 `{"created": n, "existing": m, "room_ids": [...]}`。

### 2.3 既有写接口（前端保存链路直接复用）

- `PUT /admin/schedules/{id}/weekly-slots`：body `{ "slots": [NewWeeklySlot…] }`（全量替换，服务端重建）。
- `POST /admin/schedules/{id}/rebuild`：手动重建未来实例。
- `POST /admin/schedules`：仅在 ensure 不可用时兜底（v1 不用）。

## 3. 前端共享层（已由 Lead 写好，不要重复实现）

- `frontend/src/features/scheduleBoard/boardTypes.ts`：
  - 类型：`BoardPayload/BoardArea/BoardRoom/BoardDeviceInfo/BoardSchedule/BoardWeeklySlot/BoardException/
    BoardExceptionSlot/BoardBlock/BoardConflict/BoardPeriod/BoardViewMode`；
  - 常量：`BOARD_PERIODS`（5 个固定节次）、`DEFAULT_SCHEDULE_DAYS`；
  - 纯函数：`expandBoardBlocks(payload, from, to)`（周模板+单日覆盖 → 具体日期课块）、
    `attachBoardConflicts(blocks)`（房间冲突 error / 教师冲突 warning，回写 `block.conflicts`，返回 `BoardConflict[]`）、
    `blocksOfDate/blocksOfWeek`、`weekStartOf/weekDates/addDays/isoWeekday/formatMonthDay/formatWeekRange`、
    `periodIndexOf/periodPlacement`、`groupRoomsByArea/sortRooms`、`courseColor/weekdayName`。
- `frontend/src/features/scheduleBoard/ScheduleBlockChip.vue`：课块视觉（课程色、调课/单日徽标、冲突红/琥珀样式）。
  props：`block: BoardBlock`、`compact?`、`showRoom?`、`selected?`、`dragging?`。
- `frontend/src/api/index.ts` 已新增：`api.scheduleBoard({from,to,area_id})`、`api.ensureSchedules({room_ids,start_date,end_date,name?})`。
- `frontend/src/router/routes.ts` 已新增：`/schedules` → `ScheduleBoardView.vue`（name `schedules`）、
  `/schedules/list` → `SchedulesView.vue`（name `schedule-list`，`activeName: 'schedules'`）。

## 4. 组件接口

### 4.1 `WeekGridView.vue`（前端核心负责）

props：`{ weekStart: string; rooms: BoardRoom[]; blocks: BoardBlock[]; selectedKey: string | null; canManage: boolean }`
emits：`select(block)`、`create({ room_id: string | null; date: string; start_time: string; end_time: string })`、
`move({ block: BoardBlock; day_of_week: number; start_time: string; end_time: string })`（跨天/改节次；房间不变）。
布局：列 = 一周 7 天，行 = `BOARD_PERIODS`；全部房间聚合时逐格列课块（≤3 个 + `+N`，点击 `+N` 展开浮层可选）；
「房间」筛选后只显示该房间；空格点击 `create`；课块点击 `select`；支持原生拖拽（`draggable`）落格改 `day_of_week/start_time`。

### 4.2 `ResourceTimelineView.vue`（视图负责）

props：`{ date: string; rooms: BoardRoom[]; blocks: BoardBlock[]; selectedKey: string | null; canManage: boolean }`
emits：`select(block)`、`create({ room_id: string; date: string; start_time: string; end_time: string })`、
`move({ block: BoardBlock; room_id: string; start_time: string; end_time: string })`。
布局：行 = 房间（按区域分组的粘性组头，组头显示 `area 区域 · N 间教室 · 今日 M 节`），X = 08:00–21:00
（整点刻度，默认滚动到 07:30 附近）；课块绝对定位（`left = 分钟比例`），块内用 `ScheduleBlockChip`；
拖拽（Pointer Events 或原生 DnD）横向改时间（吸附 10 分钟）+ 纵向改房间；非法/冲突落点红色反馈；
点击空白（行内轨道）emit `create`（房间 + 时间按落点，默认 100 分钟）；点击课块 emit `select`。
数据为空时给空态，不要空白画布。

### 4.3 `AllRoomMatrixView.vue`（视图负责）

props：`{ weekStart: string; rooms: BoardRoom[]; blocks: BoardBlock[]; selectedKey: string | null; canManage: boolean }`
emits：`select(block)`、`create({ room_id: string; date: string; period_index: number })`、
`move({ block: BoardBlock; room_id: string; day_of_week: number; period_index: number })`。
布局：行 = 房间（左粘性列：房间码+名称+容量+设备在线点），列 = 周一~周日 × `BOARD_PERIODS`（双层表头，
横向滚动；默认定位到周一，提供「周几」快捷跳转或在表头显示）；格内课块用 `ScheduleBlockChip compact`；
空格点击 emit `create`；课块可拖拽到其它格 emit `move`（跨天/跨节次/换房间）。
必须处理横向滚动时行头/表头粘性、1440/1280 宽度下无整页横向溢出。

## 5. 页面容器职责（前端核心负责 `ScheduleBoardView.vue`）

- 状态：`viewMode`（week/timeline/matrix）、`weekStart`、`selectedDate`、`areaId`、`roomId`、`keyword`、`onlyConflicts`、
  `selectedBlock`、`drawerOpen`、`editorState`。URL query 同步（`view/week/date/area/room`）可选但推荐。
- 加载：`api.scheduleBoard({ from: weekStart, to: weekStart+6, area_id: areaId || undefined })`；
  失败显示错误条 + 重试；loading 骨架。
- 派生：`blocks = expandBoardBlocks(payload, weekStart, weekStart+6)`；`conflicts = attachBoardConflicts(blocks)`；
  按视图/筛选过滤后传给子组件。
- 顶部工具栏（`BoardToolbar.vue`）：视图切换、周导航（‹ 今天 › + 范围文案）、区域/房间下拉、搜索框、
  「只看冲突」开关、冲突计数 chip（点击定位/简单列表）、刷新、`课表列表`链接、`新建课程`按钮。
- 右侧抽屉（`BlockDrawer.vue`）：课程/教师/房间/日期/时间/来源（周模板或单日覆盖）/冲突说明/调课徽标；
  按钮：编辑、删除、关闭；无 manage 权限时只读。宽度 ~360px，覆盖在画布右侧（不用独立路由）。
- 新建/编辑弹窗（`BlockEditModal.vue`）：课程名称（必填）、教师、房间（必填）、星期（1–7）、开始/结束时间
  （`HH:MM`，开始<结束）、备注；编辑时房间可改（= 调课，保存到另一张课表）。
- 保存链路（每个操作立即保存、成功后刷新；按钮 loading + toast）：
  1. 找到目标房间的课表：payload.schedules 中该 room 的 active 课表；没有则 `api.ensureSchedules` 创建
     （`start_date = weekStart`，`end_date = addDays(weekStart, 180)`，`name='默认课表'`）；
  2. 读该课表现有 slots（payload.weekly_slots 按 schedule_id 过滤），增/改/删目标 slot 后
     全量 `api.replaceWeeklySlots(scheduleId, slots)`；
  3. `api.rebuildSchedule(scheduleId)`；
  4. 重新 `load()` 刷新画布；失败 toast 并重新 load 兜底。
  - 跨房间移动 = 源课表删一条 + 目标课表加一条（两次 replace + 两次 rebuild）。
  - 移动保留时长：目标时间按视图给的时间为准；A 的节次拖拽若时长 > 目标节次长度，允许溢出（不截断）。
- 统计信息（底部一行可选）：本周课次、冲突数、未排课房间数。

### 5.1 单日调整（v1.5）

> 状态：**v1.5 已实现**（2026-11-01）。不改数据库/后端代码，只复用既有
> `PUT /admin/schedules/{id}/exceptions/{date}` 与 `DELETE /admin/schedules/{id}/exceptions/{date}`；
> 写成功后服务端自动重建未来实例，前端不再额外调 rebuild。

**作用域与入口**

- **B 资源时间轴默认「本次」**：拖拽课块（改房间/改时间）、点击空白新建、「本次停课」都只调整
  `block.date` 这一天，写单日覆盖（exception），**不改周模板**；工具栏「拖拽/新建作用域：本次 | 模板」
  可切到模板语义。
- **A 周网格 / C 编排矩阵固定「模板」**：保持 v1 行为（改周模板，影响整个生效区间），不显示作用域控件。
- 切到 B 时作用域自动置为「本次」，切回 A/C 自动置为「模板」。
- 右侧抽屉：模板块在 B 下多出「本次停课」；单日覆盖块（exception）多出「恢复本次」。
- 整天停课后画布内没有可点的课块，B 视图画布上方会出现「当天停课 · 恢复本次」提示条。

**写入模型（exception 写法）**

后端 `cancel` = 当天整天停课、`replace` = 当天整套时段替换，且同一课表同一天最多一条覆盖，
因此前端统一按「当天有效时段列表 → 增/删/改 → 与周模板比较」生成计划：

1. 取当天有效时段：有 exception 时 `cancel` → `[]`、`replace` → 覆盖的完整时段；否则取周模板 `day_of_week` 时段；
2. 对列表做增（新建）/删（本次停课/移动离开）/改（移动/编辑）后，与**周模板当天时段**按标准化字段
   （trim 课程/教师/备注，`room_id` 语义一致：null = 课表所属房间）比较：
   - 与模板完全一致 → `DELETE …/exceptions/{date}`（删除当天覆盖 = 恢复模板）；
   - 为空 → `PUT …/exceptions/{date}` body `{kind:'cancel', slots:[]}`（整天停课）；
   - 其它 → `PUT …/exceptions/{date}` body `{kind:'replace', slots:[完整当天时段…]}`。
3. 「本次停课」只移除该课块：当天还有其它时段 → `replace` 剩余时段；移除后为空 → `cancel`。
4. 「恢复本次」= 把该课表当天恢复为周模板（可能一次恢复当天多节课）。若是**跨房间单日调课**产生的目标课块，
   还会按「同一日期、另一课表有覆盖、当天有效时段缺该课、周模板当天含该课、模板房间与当前房间不同」识别源课表，
   并把源课表当天的这节课加回原位，避免出现「恢复后两边都没有这节课」。
5. 前端先用 `validateDaySlots` 校验（开始<结束、同课表当天时段不重叠、≤100 条），失败 toast 并拒绝，
   避免后端 400；后端仍会二次校验。

**跨房间单日移动**

- 源课表当天移除该节、目标课表当天加入该节（保留课程/教师/备注，时间取拖拽落点），
  各自按上面的规则生成计划：源/目标各一条（源可能 `delete` 恢复、目标通常 `replace`；目标房间=课表房间时
  `slots[].room_id = null`）。
- 目标房间没有 active 课表时先 `POST /admin/schedules/ensure` 补默认课表（范围 `weekStart … weekStart+180`，
  与 v1 相同）；若 `block.date` 不在目标课表生效区间内，toast 提示先到「课表列表」调整有效期，不写任何数据。
- 单日移动只在同一日期内（B 拖拽不跨日期）；跨天只走 A/C 的模板语义。

**过去日期限制**

- `date < 今天` 的单日调整（拖拽/新建/停课/恢复）一律拒绝：toast「过去日期不可调整」，不写任何数据。
  模板语义（A/C 或 B 切到「模板」）不受此限制。

**权限与保存链路**

- 仍由 `schedule:manage` 控制写入口；无权限时抽屉只读（只有「关闭」）。
- 保存链路保持 v1 的「即时保存 → toast → `load()` 刷新」；`saving` 期间禁用重复操作/拖拽，失败 toast 后
  `load()` 兜底。v1.6 起支持撤销/重做，见 §5.2。

### 5.2 撤销/重做（v1.6）

> 状态：**v1.6 已实现**（2026-11-01）。纯前端历史栈，不新增后端接口；所有写操作仍是「即时保存 + toast + reload」。

**可逆范围与快照**

- 模板类（A/C 及 B 的「模板」作用域）：新建、编辑、拖拽移动、删除。操作前对受影响课表
  `snapshotTemplates(payload, scheduleIds)` 取 `NewWeeklySlot[]` 快照；跨房间移动会同时快照源/目标两张课表。
- 单日类（B 的「本次」作用域）：本次新建、本次拖拽调课（含跨房间两条计划）、本次停课、恢复本次、整天停课恢复。
  操作前对相关 `(schedule_id, date)` 调用 `snapshotExceptions(payload, refs)`：
  - 当天有覆盖 → 记录 `kind/note/slots`（`NewExceptionSlot[]`）；
  - 当天无覆盖 → 记录 `kind: null`。
  - refs 由 `exceptionPlanRefs(plans)` 从 `ExceptionPlan[]` 去重得到。
- 不记录纯只读操作（导航、筛选、刷新、选中、打开抽屉）。

**历史栈**

- `undoStack` / `redoStack`：每项 `{ label, undo: () => Promise<void>, redo: () => Promise<void> }`，
  快照在「操作前」抓取，计划在「操作时」生成；最多 50 步，超出丢弃最旧一项。
- 每次写操作**成功后** push 到 `undoStack` 并清空 `redoStack`；`label` 用作 toast 与按钮 title
  （新建课程/编辑课程/移动课程/删除课程/本次新建/本次调课/本次停课/恢复本次/恢复当天停课）。
- 撤销：`undoStack` 弹出一项 → 执行 `undo`：
  - 模板：按快照对每张课表全量 `PUT /weekly-slots` + `POST /rebuild`（复用 v1 保存链路）；
  - 单日：`kind === null` → `DELETE /exceptions/{date}`，否则 `PUT /exceptions/{date}` body `{kind, note, slots}`。
  成功后该项进入 `redoStack`，toast「已撤销：<label>」。
- 重做：`redoStack` 弹出一项 → 重放本次计划（模板 = `SlotReplacePlan[]` 的 replace/rebuild；单日 = `ExceptionPlan[]`
  的 delete/cancel/replace），成功后放回 `undoStack`，toast「已重做：<label>」。
- 撤销/重做期间复用 `saving` 锁（按钮与快捷键都忽略重复触发），执行后 `load()` 刷新；失败 toast + `load()` 兜底，
  该项保留在栈中可重试。撤销导致当前选中课块不存在时，`load()` 会自动关闭抽屉。

**入口与快捷键**

- 工具栏刷新按钮左侧：`rotate-ccw`（撤销）/ `rotate-cw`（重做）两个 `IconButton`；
  无历史时 disabled；title 为「撤销：<label>（Ctrl+Z）」/「重做：<label>（Ctrl+Shift+Z）」。
- `Ctrl/Cmd+Z` 撤销；`Ctrl/Cmd+Shift+Z` 或 `Ctrl/Cmd+Y` 重做。
- 焦点在 `input/textarea/select`（含 contenteditable）时不拦截浏览器默认行为；新建/编辑弹窗、删除确认弹窗打开时
  也不拦截；页面卸载时移除 `keydown` 监听。

**已知边界**

- 历史只存在于当前页面会话：刷新/离开页面即丢失（不做持久化）。
- 模板快照只含 weekly slots，不包含课表本身：若某次操作通过 `ensure` 新建了课表，撤销后课表会保留（变为空课表）。
- 跨房间写入仍是两次独立请求（非事务），撤销/重做同样可能部分成功，失败以 `load()` 兜底后可重试。

### 5.3 框选批量应用/清除（v1.7）

> 状态：**v1.7 已实现**（2026-11-01）。仅 C 编排矩阵；纯前端框选手势 + 批量周模板写入，不新增后端接口。

**框选交互（`AllRoomMatrixView.vue`）**

- 在矩阵格子上按下并拖拽 → 矩形框选（跨房间行、跨天/跨节次列）；拖动中实时高亮（accent 半透明 + ring），
  松开后选区保留并 emit `update:selection`（`MatrixCellRef[] = {room_id, day_of_week, period_index}`，
  `period_index` 与 `periodPlacement(block.start_time).index` 同一套节次下标）。
- **简单点击**（按下后移动 < 4px）不产生选区，仍保持原「点空格新建」行为；拖拽结束后短暂抑制 click，
  避免误触新建。
- 在课块上按下不启动框选（`data-block-key` 命中即返回），保留课块拖拽；只读账号（`canManage=false`）不启用框选。
- `Esc` 清空选区；切换视图 / 星期 / 区域时页面清空选区（组件在 `weekStart` 变化时也会清空）；
  `saving` 期间矩阵 `canManage` 为 false，框选与操作按钮同时禁用。

**页面操作条（`ScheduleBoardView.vue`）**

- 仅 `viewMode === 'matrix'` 且选区非空时显示：「已选 N 格（跨 M 间教室） · [批量应用课程] [批量清除] [取消选择]」；
  N = 选区格子数，M = 去重房间数。
- 批量应用弹窗 `BatchFillModal.vue`：课程名称（必填）/ 教师 / 备注 + 单选「仅填充空格（默认）」/「覆盖已有课」；
  实时预览「将新建 X 节 · 跳过 Y 节 · 涉及 Z 张课表」；同一天时段重叠或超过 100 条时展示错误并禁用提交。

**批量计划纯函数（`boardActions.ts`）**

- `planBatchFill(payload, cells, value, mode)`：
  - 格子按房间去重、按 `(day_of_week, period_index)` 去重；时段固定取 `BOARD_PERIODS[period_index]`
    （`start_time/end_time` = 节次起止），`room_id` 写 `null`（= 课表所属房间）；
  - 占用判定 = 该课表同一天且 `periodIndexOf(slot.start_time) === period_index`；`empty` 模式跳过占用格，
    `overwrite` 先移除占用 slot 再追加；
  - 返回 `{plans, created, skipped, perRoom, errors}`；每个有变化的课表一条 `SlotReplacePlan`；
    房间还没有课表时 `perRoom.schedule_id = ''`、按「空格子」计入 created，但不产出计划（等待 ensure 后重算）；
  - `errors` 校验批量涉及的星期：时段重叠、同一天超过 100 条。
- `planBatchClear(payload, cells)`：移除选中格对应「同课表 + 同星期 + 同节次」的全部周模板 slot，
  返回 `{plans, cleared, perRoom, errors}`；没有可清除课程时 `plans=[]`。

**执行链路**

1. 批量应用：按选区房间去重 → 缺 active 课表的房间先 `POST /schedules/ensure`
   （`weekStart … weekStart+180`，与 v1 一致）→ `load()` 后**重新计划** → 每个课表
   `PUT /weekly-slots`（全量）+ `POST /rebuild` → toast「已批量新建 X 节（跳过 Y）」→ 清空选区 → `load()`。
2. 批量清除：`ConfirmDialog` 二次确认 → `planBatchClear` → 同样按课表全量 replace + rebuild →
   toast「已清除 X 节」→ 清空选区 → `load()`。
3. **撤销/重做**：一次批量操作只压入一条历史（`批量应用课程` / `批量清除`）；执行前对涉及课表做
   `snapshotTemplates`，undo = 恢复快照（全量 replace + rebuild），redo = 重放批量 `plans`（复用 v1.6 机制）。
4. 失败 toast + `load()` 兜底；`saving` 期间禁用框选、弹窗提交与操作条按钮。

**已知边界**

- 批量只改周模板（weekly_slots），不写/不删单日覆盖；有单日覆盖的日期仍以覆盖为准。
- 同一房间存在多份 active 课表时沿用 `scheduleOfRoom`（`start_date` 最早的一份）语义。
- 框选拖拽不自动横向滚动：较长选区需先滚动矩阵再拖拽；选区只覆盖当前可见矩阵的格子模型（不依赖 DOM 可见性，
  但拖拽落点必须在当前视口内）。
- 选区不跨周持久化；切换周/区域/视图即清空。

### 5.4 C 矩阵矩形复制/粘贴（v1.8）

> 状态：**v1.8 已实现**（2026-11-01）。仅 C 编排矩阵；模板语义、纯前端剪贴板 + 周模板批量写入，不新增后端接口。

**复制（Copy）**

- 框选后操作条新增「复制」；也可 `Ctrl/Cmd+C`（`input/textarea/select` 内不拦截）。
- 点击房间行头（左侧粘性房间列）可选中/取消该房间整行（7 天 × 5 节次 = 35 格），方便整周复制。
- 剪贴板结构 `MatrixClipboard`：`rows`（按矩阵排序的房间 id）、`columns`（`{day_of_week, period_index}` 相对矩形列）、
  `cells`（只含有课块的源格：相对 `row/col` + 该格全部周模板 slot）。
- 只复制**周模板课块**：当天存在单日覆盖（cancel/replace）的格子按「非模板」跳过；同格多块全部复制
  （不受 C 格显示上限影响）。空格子不进入 `cells`。
- 复制后操作条切换为「已复制 N 格（跨 M 间教室） · [粘贴] [取消复制]」，N = 矩形格子数（rows×columns）、M = 行数；
  选区内没有可复制课程时「复制」禁用；**选区变化后旧剪贴板自动失效**。

**粘贴（Paste）**

1. 点「粘贴」进入粘贴模式：矩阵显示提示「点击目标格粘贴（Esc 取消）」，hover 时按剪贴板 rows×columns
   显示虚线目标矩形。
2. 点击任意格子作为**锚点**（矩形左上角）；按 `锚点行 + 剪贴板相对 row`、`锚点列 + 相对 col` 计算目标格，
   房间行 / 7 天×5 节次列越界则跳过（计入 skipped）。
3. 粘贴规则：
   - 目标格先移除「同房间 + 同 day 且 `periodIndexOf(slot.start_time)` 命中目标节次」的现有模板 slot；
   - 再写入复制块：时间 = `目标节次起点 + (源块起点 - 源节次起点)`，时长不变（平移越界计入 skipped）；
   - 课程/教师/备注原样复制，`room_id` 写 `null`（目标房间 = 课表房间）；
   - 源格为空（不在 `cells` 中）→ 目标格不动，不清空。
4. 执行前 `ConfirmDialog` 预览：「将粘贴 X 节课程，覆盖目标格 Y 节课程，涉及 Z 张课表」；
   若同一天时段重叠或超过 100 条 → toast 错误并取消粘贴（退出粘贴模式）。
5. 确认后：目标房间缺课表先 `POST /schedules/ensure`（范围 `weekStart … weekStart+180`）→ `load()` 重新计划 →
   每课表 `PUT /weekly-slots`（全量）+ `POST /rebuild` → toast「已粘贴 X 节课程（覆盖 Y 节）」→
   清空剪贴板/选区/粘贴模式 → `load()`。
6. **撤销/重做**：一次粘贴只压入一条历史（label「粘贴课程」）；操作前 `snapshotTemplates` 涉及课表，
   undo = 恢复快照，redo = 重放 `plans`（复用 v1.6 机制）。
7. `Esc` 依次退出粘贴模式 / 清空选区；切视图 / 星期 / 区域清空选区、粘贴模式与剪贴板；
   `saving` 期间禁用复制/粘贴/框选；只读账号（无 `schedule:manage`）不显示复制/粘贴入口。

**纯函数与类型（`boardActions.ts`）**

- `MatrixClipboard`、`MatrixClipboardCell`、`MatrixClipboardSlot`；
- `buildMatrixClipboard(payload, rooms, cells)`：按矩阵行顺序（`sortRooms`）与列顺序（day × period）构建相对矩形，
  rows/columns 覆盖选区矩形范围，cells 仅保留可复制的模板课块；
- `planBatchPaste(payload, clipboard, anchorCell, rooms)` → `BatchPasteResult {plans, created, overwritten, skipped, perRoom, errors}`；
  每个有变化的课表一条 `SlotReplacePlan`；目标房间无课表时 `perRoom.schedule_id = ''` 并按新建计入 created，
  待页面 ensure 后重算；`errors` 校验涉及星期的重叠/超 100 条。

**已知边界**

- 仅模板语义：不复制/不删除单日覆盖；当天有覆盖的源格不进入剪贴板。
- 行头整行选择 = 当前矩阵 7 天 × 5 节次 = 35 格（早期文档写作 25 格是笔误）；复制/粘贴不跨周持久化。
- 目标课表存在多份 active 课表时沿用 `scheduleOfRoom`（`start_date` 最早）语义。
- 时间平移越界（如源时段偏移过大导致跨天）按 skipped 跳过，不写入。

## 6. 验证与命令

- 前端：`cd frontend && ./node_modules/.bin/vue-tsc --noEmit -p tsconfig.app.json`（并发安全）。
- 后端：`cargo check -p qe-api --all-targets`、`cargo fmt --check`；必要时 `cargo test -p qe-domain`。
- 真实联调：后端二进制在 `127.0.0.1:8080`，前端 Vite 在 `127.0.0.1:5174`（已在运行，`/api` 代理到 8080，
  改动后端需重新 build 并重启 8080 进程，重启由 Lead/QA 统一做，不要各自杀进程）。
- 开发账号见 `frontend/docs/CONVENTIONS.md`；测试数据加 `dev-` 前缀并清理。
