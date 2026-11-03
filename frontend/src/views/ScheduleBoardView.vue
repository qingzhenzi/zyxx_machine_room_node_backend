<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import AllRoomMatrixView from '@/features/scheduleBoard/AllRoomMatrixView.vue'
import BatchFillModal from '@/features/scheduleBoard/BatchFillModal.vue'
import BlockDrawer from '@/features/scheduleBoard/BlockDrawer.vue'
import BlockEditModal from '@/features/scheduleBoard/BlockEditModal.vue'
import BoardToolbar from '@/features/scheduleBoard/BoardToolbar.vue'
import ResourceTimelineView from '@/features/scheduleBoard/ResourceTimelineView.vue'
import WeekGridView from '@/features/scheduleBoard/WeekGridView.vue'
import {
  BOARD_PERIODS,
  addDays,
  attachBoardConflicts,
  expandBoardBlocks,
  formatMonthDay,
  isoWeekday,
  weekStartOf,
  weekdayName,
  type BoardBlock,
  type BoardConflict,
  type BoardPayload,
  type BoardSchedule,
  type BoardViewMode,
} from '@/features/scheduleBoard/boardTypes'
import {
  addMinutes,
  buildMatrixClipboard,
  defaultScheduleRange,
  durationMinutes,
  emptySlotForm,
  planBatchClear,
  planBatchFill,
  planBatchPaste,
  exceptionPlanRefs,
  formValueOfBlock,
  isTimeString,
  minutesOf,
  moveFormOfBlock,
  planCreateSlot,
  planDeleteSlot,
  planOccurrenceCancel,
  planOccurrenceCreate,
  planOccurrenceMove,
  planOccurrenceRestore,
  planUpdateSlot,
  scheduleById,
  scheduleOfRoom,
  snapshotExceptions,
  snapshotTemplates,
  validateDaySlots,
  validateSlotForm,
  type BatchFillMode,
  type BatchFillValue,
  type BatchPasteResult,
  type BoardDaySlot,
  type BoardSlotFormValue,
  type ExceptionPlan,
  type ExceptionSnapshot,
  type MatrixCellRef,
  type MatrixClipboard,
  type SlotReplacePlan,
} from '@/features/scheduleBoard/boardActions'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { today } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

interface CreateRequest {
  room_id: string | null
  date: string
  start_time?: string
  end_time?: string
  period_index?: number
}

interface MoveRequest {
  block: BoardBlock
  room_id?: string
  day_of_week?: number
  period_index?: number
  start_time?: string
  end_time?: string
}

const DATE_PATTERN = /^\d{4}-\d{2}-\d{2}$/

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()
const toast = useToastStore()

const canManage = computed(() => auth.can(PERMISSIONS.scheduleManage))

function queryString(value: unknown): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value) && typeof value[0] === 'string') return value[0]
  return ''
}

function parseViewQuery(value: unknown): BoardViewMode {
  const raw = queryString(value)
  return raw === 'timeline' || raw === 'matrix' ? raw : 'week'
}

function parseDateQuery(value: unknown): string | null {
  const raw = queryString(value)
  return DATE_PATTERN.test(raw) ? raw : null
}

function clampToWeek(date: string, start: string): string {
  const end = addDays(start, 6)
  if (date < start) return start
  if (date > end) return end
  return date
}

const initialDate = parseDateQuery(route.query.date)
const initialWeek = parseDateQuery(route.query.week)

const viewMode = ref<BoardViewMode>(parseViewQuery(route.query.view))
const weekStart = ref(weekStartOf(initialWeek ?? initialDate ?? today()))
const selectedDate = ref(
  initialDate ?? (initialWeek ? initialWeek : clampToWeek(today(), weekStart.value)),
)

const areaId = ref(queryString(route.query.area))
const roomId = ref(queryString(route.query.room))
const keyword = ref('')
const onlyConflicts = ref(false)

const payload = ref<BoardPayload | null>(null)
const loading = ref(false)
const error = ref('')

const selectedKey = ref<string | null>(null)
const drawerOpen = ref(false)

const editorOpen = ref(false)
const editorMode = ref<'create' | 'edit'>('create')
const editorInitial = ref<BoardSlotFormValue | null>(null)
const editorError = ref('')
const editingBlock = ref<BoardBlock | null>(null)
const saving = ref(false)

const deleteTarget = ref<BoardBlock | null>(null)
const deleting = ref(false)

/** B 时间轴的调整作用域（本次 = 写单日覆盖；模板 = 改周模板）；A/C 固定模板。 */
const editScope = ref<'template' | 'occurrence'>(
  parseViewQuery(route.query.view) === 'timeline' ? 'occurrence' : 'template',
)
/** 新建弹窗打开时的落库场景（作用域 + 目标日期），保存时决定写模板还是写单日覆盖。 */
const createScene = ref<{ scope: 'template' | 'occurrence'; date: string } | null>(null)

interface HistoryEntry {
  label: string
  undo: () => Promise<void>
  redo: () => Promise<void>
}

const HISTORY_LIMIT = 50
/** 撤销/重做栈：每项保存动作名 + 逆操作/重放（快照在操作前抓取）。 */
const undoStack = ref<HistoryEntry[]>([])
const redoStack = ref<HistoryEntry[]>([])
const canUndo = computed(
  () => canManage.value && !saving.value && !loading.value && undoStack.value.length > 0,
)
const canRedo = computed(
  () => canManage.value && !saving.value && !loading.value && redoStack.value.length > 0,
)
const undoLabel = computed(() => undoStack.value[undoStack.value.length - 1]?.label ?? '')
const redoLabel = computed(() => redoStack.value[redoStack.value.length - 1]?.label ?? '')

/** C 矩阵框选选区（仅 matrix 视图生效）与批量操作弹窗/确认状态。 */
const matrixSelection = ref<MatrixCellRef[]>([])
const matrixSelectedRoomCount = computed(
  () => new Set(matrixSelection.value.map((cell) => cell.room_id)).size,
)
const batchFillOpen = ref(false)
const batchClearOpen = ref(false)
const batchFillError = ref('')

/** C 矩阵复制/粘贴：剪贴板 + 「粘贴就绪 / 粘贴模式 / 确认预览」状态。 */
const matrixClipboard = ref<MatrixClipboard | null>(null)
const clipboardSelection = ref<MatrixCellRef[] | null>(null)
const pasteMode = ref(false)
const pasteConfirmOpen = ref(false)
const pendingPasteAnchor = ref<MatrixCellRef | null>(null)
const pendingPastePreview = ref<BatchPasteResult | null>(null)
const clipboardCellCount = computed(() => {
  const clipboard = matrixClipboard.value
  return clipboard ? clipboard.rows.length * clipboard.columns.length : 0
})
const clipboardRoomCount = computed(() => matrixClipboard.value?.rows.length ?? 0)

const blockData = computed(() => {
  if (!payload.value) return { blocks: [] as BoardBlock[], conflicts: [] as BoardConflict[] }
  const from = weekStart.value
  const blocks = expandBoardBlocks(payload.value, from, addDays(from, 6))
  const conflicts = attachBoardConflicts(blocks)
  return { blocks, conflicts }
})

const allBlocks = computed(() => blockData.value.blocks)
const conflicts = computed(() => blockData.value.conflicts)

const areaRooms = computed(() => {
  const rooms = payload.value?.rooms ?? []
  return areaId.value ? rooms.filter((room) => room.area_id === areaId.value) : rooms
})

const visibleRooms = computed(() =>
  roomId.value ? areaRooms.value.filter((room) => room.id === roomId.value) : areaRooms.value,
)

/** 当前选区内是否有可复制的周模板课块（决定「复制」按钮是否可用）。 */
const matrixCopyable = computed(
  () =>
    canManage.value &&
    matrixSelection.value.length > 0 &&
    buildMatrixClipboard(payload.value, visibleRooms.value, matrixSelection.value).cells.length > 0,
)

/** 区域/房间范围内的课块（不受搜索与只看冲突影响），用于统计。 */
const scopeBlocks = computed(() =>
  allBlocks.value.filter(
    (block) =>
      (!areaId.value || block.area_id === areaId.value) &&
      (!roomId.value || block.room_id === roomId.value),
  ),
)

/** 传给三个视图的课块：区域/房间 + 关键词 + 只看冲突。 */
const visibleBlocks = computed(() => {
  const needle = keyword.value.trim().toLowerCase()
  return scopeBlocks.value.filter((block) => {
    if (onlyConflicts.value && block.conflicts.length === 0) return false
    if (!needle) return true
    return `${block.course_name} ${block.teacher} ${block.room_code} ${block.room_name}`
      .toLowerCase()
      .includes(needle)
  })
})

const visibleConflicts = computed(() => {
  const keys = new Set(scopeBlocks.value.map((block) => block.key))
  return conflicts.value.filter((conflict) => conflict.block_keys.some((key) => keys.has(key)))
})

const selectedBlock = computed(
  () => allBlocks.value.find((block) => block.key === selectedKey.value) ?? null,
)

const dragEnabled = computed(() => canManage.value && !saving.value)

/** 单日动作只开放给 B 时间轴（过去日期在点击时再拦截并 toast）。 */
const canAdjustOccurrence = computed(() => canManage.value && viewMode.value === 'timeline')

/** B 视图当天被整天停课的课表（用于在画布上方提供「恢复本次」入口）。 */
const cancelledDaySchedules = computed<BoardSchedule[]>(() => {
  const data = payload.value
  if (!data || viewMode.value !== 'timeline') return []
  const roomIds = new Set(visibleRooms.value.map((room) => room.id))
  const schedules: BoardSchedule[] = []
  for (const exception of data.exceptions) {
    if (exception.kind !== 'cancel' || exception.exception_date !== selectedDate.value) continue
    const schedule = data.schedules.find((item) => item.id === exception.schedule_id)
    if (!schedule || !roomIds.has(schedule.room_id)) continue
    schedules.push(schedule)
  }
  return schedules
})

const cancelledDayLabel = computed(() =>
  cancelledDaySchedules.value
    .map((schedule) => {
      const room = payload.value?.rooms.find((item) => item.id === schedule.room_id)
      return room ? room.code : schedule.room_id.slice(0, 8)
    })
    .join('、'),
)

const unassignedRooms = computed(
  () =>
    visibleRooms.value.filter(
      (room) => !scopeBlocks.value.some((block) => block.room_id === room.id),
    ).length,
)

const emptyHint = computed<{ title: string; description: string; action: string } | null>(() => {
  if (!payload.value) return null
  if (!payload.value.rooms.length) {
    return {
      title: '所选区域还没有可用房间',
      description: '请先在「房间」页面创建并启用房间，再回到排课工作台。',
      action: 'none',
    }
  }
  if (!payload.value.schedules.length) {
    return {
      title: areaId.value ? '所选区域还没有课表' : '还没有课表',
      description: '为房间新建课程时会自动补一张「默认课表」。',
      action: canManage.value ? 'create' : 'none',
    }
  }
  if (!allBlocks.value.length) {
    return {
      title: '本周还没有排课',
      description: '点击周网格空白格或右上角「新建课程」开始编排。',
      action: canManage.value ? 'create' : 'none',
    }
  }
  if (!visibleBlocks.value.length) {
    return {
      title: '没有符合筛选条件的课程',
      description: '试试清除搜索关键词、房间筛选或关闭「只看冲突」。',
      action: 'reset',
    }
  }
  return null
})

const headerDescription = computed(() => {
  if (!payload.value?.rooms.length) return '跨房间编排课程：周模板、单日覆盖与实时冲突检测。'
  const parts = [
    `跨 ${payload.value.rooms.length} 个房间编排课程`,
    `本周 ${scopeBlocks.value.length} 节课`,
    `${visibleConflicts.value.length} 处冲突`,
  ]
  if (unassignedRooms.value) parts.push(`${unassignedRooms.value} 间房间无课`)
  return `${parts.join(' · ')}。`
})

function actionError(caught: unknown, fallback: string): string {
  return isForbidden(caught) ? '缺少课表管理权限' : errorMessage(caught, fallback)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.scheduleBoard({
      from: weekStart.value,
      to: addDays(weekStart.value, 6),
      area_id: areaId.value || undefined,
    })
    payload.value = result
    if (roomId.value && !result.rooms.some((room) => room.id === roomId.value)) {
      roomId.value = ''
    }
    if (
      selectedKey.value &&
      !blockData.value.blocks.some((block) => block.key === selectedKey.value)
    ) {
      selectedKey.value = null
      drawerOpen.value = false
    }
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载排课工作台失败')
    payload.value = null
  } finally {
    loading.value = false
  }
}

function shiftWeek(delta: number) {
  weekStart.value = addDays(weekStart.value, delta * 7)
  selectedDate.value = clampToWeek(selectedDate.value, weekStart.value)
}

function goToday() {
  const current = today()
  weekStart.value = weekStartOf(current)
  selectedDate.value = current
}

function shiftDay(delta: number) {
  const next = addDays(selectedDate.value, delta)
  selectedDate.value = next
  weekStart.value = weekStartOf(next)
}

function resetFilters() {
  keyword.value = ''
  onlyConflicts.value = false
  roomId.value = ''
}

function onSelect(block: BoardBlock) {
  selectedKey.value = block.key
  drawerOpen.value = true
}

function closeDrawer() {
  drawerOpen.value = false
  selectedKey.value = null
}

function openCreate(prefill?: Partial<BoardSlotFormValue>) {
  if (!canManage.value || saving.value) return
  const base = emptySlotForm()
  editorMode.value = 'create'
  editingBlock.value = null
  editorError.value = ''
  editorInitial.value = {
    ...base,
    room_id: prefill?.room_id ?? roomId.value ?? '',
    ...prefill,
  }
  editorOpen.value = true
}

function onCreate(request: CreateRequest) {
  if (!canManage.value || saving.value) return
  const occurrenceScope = viewMode.value === 'timeline' && editScope.value === 'occurrence'
  if (occurrenceScope && rejectPastDate(request.date)) return
  let start = request.start_time
  let end = request.end_time
  if (!start || !end) {
    const period = BOARD_PERIODS[request.period_index ?? 0] ?? BOARD_PERIODS[0]!
    start = period.start
    end = period.end
  }
  createScene.value = {
    scope: occurrenceScope ? 'occurrence' : 'template',
    date: request.date,
  }
  openCreate({
    room_id: request.room_id ?? roomId.value ?? '',
    day_of_week: isoWeekday(request.date),
    start_time: start,
    end_time: end,
  })
}

function submitEditor(value: BoardSlotFormValue) {
  if (editorMode.value === 'create') void saveCreate(value)
  else void saveUpdate(value)
}

function onCreateFromToolbar() {
  if (!canManage.value || saving.value) return
  const occurrenceScope = viewMode.value === 'timeline' && editScope.value === 'occurrence'
  if (occurrenceScope && rejectPastDate(selectedDate.value)) return
  createScene.value = {
    scope: occurrenceScope ? 'occurrence' : 'template',
    date: selectedDate.value,
  }
  const day = isoWeekday(selectedDate.value)
  const period = BOARD_PERIODS[0]!
  openCreate({ day_of_week: day, start_time: period.start, end_time: period.end })
}

function openEdit() {
  const block = selectedBlock.value
  if (!block || block.source !== 'template' || !canManage.value || saving.value) return
  editorMode.value = 'edit'
  editingBlock.value = block
  editorInitial.value = formValueOfBlock(block)
  editorError.value = ''
  editorOpen.value = true
}

function requestDelete() {
  const block = selectedBlock.value
  if (!block || block.source !== 'template' || !canManage.value || deleting.value) return
  deleteTarget.value = block
}

/** 找到房间的 active 课表；没有则补默认课表，并在 ensure 后刷新一次数据。 */
async function resolveScheduleForRoom(room: string): Promise<BoardSchedule | null> {
  const existing = scheduleOfRoom(payload.value, room)
  if (existing) return existing

  const range = defaultScheduleRange(weekStart.value)
  const reply = await api.ensureSchedules({
    room_ids: [room],
    start_date: range.start_date,
    end_date: range.end_date,
    name: '默认课表',
  })
  const ensured =
    [...reply.created, ...reply.existing].find((schedule) => schedule.room_id === room) ?? null
  await load()
  return ensured ? scheduleOfRoom(payload.value, room) : null
}

async function applyPlan(plan: SlotReplacePlan[]) {
  for (const step of plan) {
    await api.replaceWeeklySlots(step.schedule_id, step.slots)
    await api.rebuildSchedule(step.schedule_id)
  }
}

function assertLoaded(...scheduleIds: string[]) {
  for (const id of scheduleIds) {
    if (!scheduleById(payload.value, id)) {
      throw new Error('课表数据尚未刷新，请重试')
    }
  }
}

function successDescription(value: BoardSlotFormValue): string {
  return `${value.course_name.trim()} · ${weekdayName(value.day_of_week)} ${value.start_time}–${value.end_time}`
}

function rejectPastDate(date: string): boolean {
  if (date >= today()) return false
  toast.error(
    '过去日期不可调整',
    `${formatMonthDay(date)} 已过去，单日调整只支持今天及以后的日期。`,
  )
  return true
}

/** 单日计划落库：delete 删覆盖 / cancel 整天停课 / replace 整套当天时段。 */
async function applyExceptionPlans(plans: ExceptionPlan[]) {
  for (const plan of plans) {
    if (plan.action === 'delete') {
      await api.deleteException(plan.schedule_id, plan.date)
    } else if (plan.action === 'cancel') {
      await api.upsertException(plan.schedule_id, plan.date, { kind: 'cancel', slots: [] })
    } else {
      await api.upsertException(plan.schedule_id, plan.date, {
        kind: 'replace',
        slots: plan.slots,
      })
    }
  }
}

/** 把单日覆盖快照写回：kind=null → 删除当天覆盖；否则整体 upsert。 */
async function applyExceptionSnapshots(snapshots: ExceptionSnapshot[]) {
  for (const snap of snapshots) {
    if (snap.kind === null) {
      await api.deleteException(snap.schedule_id, snap.date)
    } else {
      await api.upsertException(snap.schedule_id, snap.date, {
        kind: snap.kind,
        note: snap.note,
        slots: snap.slots,
      })
    }
  }
}

/** 记录一次成功的写操作：undo 恢复操作前快照，redo 重放本次计划（最多 50 步）。 */
function pushHistory(entry: HistoryEntry) {
  undoStack.value.push(entry)
  if (undoStack.value.length > HISTORY_LIMIT) undoStack.value.shift()
  redoStack.value = []
}

async function undo() {
  if (!canManage.value || saving.value || deleting.value || loading.value) return
  const entry = undoStack.value.pop()
  if (!entry) return
  saving.value = true
  try {
    await entry.undo()
    redoStack.value.push(entry)
    toast.success(`已撤销：${entry.label}`)
    await load()
  } catch (caught) {
    undoStack.value.push(entry)
    toast.error('撤销失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

async function redo() {
  if (!canManage.value || saving.value || deleting.value || loading.value) return
  const entry = redoStack.value.pop()
  if (!entry) return
  saving.value = true
  try {
    await entry.redo()
    undoStack.value.push(entry)
    if (undoStack.value.length > HISTORY_LIMIT) undoStack.value.shift()
    toast.success(`已重做：${entry.label}`)
    await load()
  } catch (caught) {
    redoStack.value.push(entry)
    toast.error('重做失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

/** 输入框/下拉框内不抢 Ctrl+Z；弹窗打开时也不拦截。 */
function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable
}

function onHistoryKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    if (pasteMode.value) {
      event.preventDefault()
      // 捕获阶段拦截：只退出粘贴模式，保留选区/剪贴板；矩阵的 Esc 处理器不再清空选区。
      event.stopPropagation()
      exitPasteMode()
    }
    return
  }
  if (!(event.ctrlKey || event.metaKey)) return
  if (editorOpen.value || deleteTarget.value || isTypingTarget(event.target)) return
  const key = event.key.toLowerCase()
  if (key === 'c') {
    if (canManage.value && viewMode.value === 'matrix' && matrixSelection.value.length) {
      event.preventDefault()
      copyMatrixSelection()
    }
    return
  }
  if (key === 'z' && event.shiftKey) {
    if (!canRedo.value) return
    event.preventDefault()
    void redo()
    return
  }
  if (key === 'z') {
    if (!canUndo.value) return
    event.preventDefault()
    void undo()
    return
  }
  if (key === 'y') {
    if (!canRedo.value) return
    event.preventDefault()
    void redo()
  }
}

/** 单日计划内 replace 的时段列表先做重叠/时间顺序校验，避免后端 400。 */
function validateExceptionPlans(plans: ExceptionPlan[]): string {
  for (const plan of plans) {
    if (plan.action !== 'replace') continue
    const slots: BoardDaySlot[] = plan.slots.map((slot) => ({
      start_time: slot.start_time,
      end_time: slot.end_time,
      course_name: slot.course_name,
      teacher: slot.teacher ?? '',
      room_id: slot.room_id ?? null,
      note: slot.note ?? '',
    }))
    const message = validateDaySlots(slots)
    if (message) return message
  }
  return ''
}

function assertScheduleCoversDate(schedule: BoardSchedule, date: string, label: string) {
  if (date >= schedule.start_date && date <= schedule.end_date) return
  throw new Error(
    `${label}课表生效区间为 ${schedule.start_date}–${schedule.end_date}，不包含 ${formatMonthDay(date)}，请先在「课表列表」调整有效期`,
  )
}

async function saveCreate(value: BoardSlotFormValue) {
  if (saving.value) return
  const scene = createScene.value
  if (scene?.scope === 'occurrence') {
    await saveCreateOccurrence(value, scene.date)
    return
  }
  saving.value = true
  editorError.value = ''
  try {
    const schedule = await resolveScheduleForRoom(value.room_id)
    if (!schedule) throw new Error('房间所属课表创建失败')
    assertLoaded(schedule.id)
    const plan = planCreateSlot(payload.value, schedule, value)
    const snapshots = snapshotTemplates(payload.value, [schedule.id])
    await applyPlan(plan)
    editorOpen.value = false
    pushHistory({
      label: '新建课程',
      undo: () => applyPlan(snapshots),
      redo: () => applyPlan(plan),
    })
    toast.success('课程已创建', successDescription(value))
    await load()
    const visible = blockData.value.blocks.some(
      (block) =>
        block.schedule_id === schedule.id && block.course_name === value.course_name.trim(),
    )
    if (!visible) {
      toast.info(
        '当前周未生成课块',
        '该房间课表的生效区间可能不包含本周，可在「课表列表」调整起止日期后重建。',
      )
    }
  } catch (caught) {
    editorError.value = actionError(caught, '保存课程失败，请稍后重试')
    toast.error('保存课程失败', editorError.value)
    await load()
  } finally {
    saving.value = false
  }
}

/** 「本次」新建：当天有效时段 + 新时段 → 与模板比较后写单日覆盖。 */
async function saveCreateOccurrence(value: BoardSlotFormValue, date: string) {
  saving.value = true
  editorError.value = ''
  try {
    const expectedDay = isoWeekday(date)
    if (value.day_of_week !== expectedDay) {
      throw new Error(
        `「本次」新建只落在 ${formatMonthDay(date)}（${weekdayName(expectedDay)}），请勿修改星期`,
      )
    }
    const schedule = await resolveScheduleForRoom(value.room_id)
    if (!schedule) throw new Error('房间所属课表创建失败')
    assertLoaded(schedule.id)
    assertScheduleCoversDate(schedule, date, '该房间')
    const dayOfWeek = isoWeekday(date)
    const plans = planOccurrenceCreate(payload.value, schedule, date, dayOfWeek, value)
    const message = validateExceptionPlans(plans)
    if (message) throw new Error(message)
    const snapshots = snapshotExceptions(payload.value, exceptionPlanRefs(plans))
    await applyExceptionPlans(plans)
    editorOpen.value = false
    pushHistory({
      label: '本次新建',
      undo: () => applyExceptionSnapshots(snapshots),
      redo: () => applyExceptionPlans(plans),
    })
    toast.success(
      '已新建本次课程',
      `${value.course_name.trim()} · ${formatMonthDay(date)} ${value.start_time}–${value.end_time}`,
    )
    await load()
  } catch (caught) {
    editorError.value = actionError(caught, '新建本次课程失败，请稍后重试')
    toast.error('新建本次课程失败', editorError.value)
    await load()
  } finally {
    saving.value = false
  }
}

async function saveUpdate(value: BoardSlotFormValue) {
  const block = editingBlock.value
  if (!block || saving.value) return
  saving.value = true
  editorError.value = ''
  try {
    const target = await resolveScheduleForRoom(value.room_id)
    if (!target) throw new Error('目标房间所属课表创建失败')
    assertLoaded(block.schedule_id, target.id)
    const plan = planUpdateSlot(payload.value, block.schedule_id, block.slot_id, target, value)
    const snapshots = snapshotTemplates(payload.value, [block.schedule_id, target.id])
    await applyPlan(plan)
    editorOpen.value = false
    closeDrawer()
    pushHistory({
      label: '编辑课程',
      undo: () => applyPlan(snapshots),
      redo: () => applyPlan(plan),
    })
    toast.success('课程已更新', successDescription(value))
    await load()
  } catch (caught) {
    editorError.value = actionError(caught, '保存修改失败，请稍后重试')
    toast.error('保存修改失败', editorError.value)
    await load()
  } finally {
    saving.value = false
  }
}

/** 「本次」拖拽：只调整 block.date 当天的这一节课；跨房间 = 源/目标课表各写一条当天覆盖。 */
async function moveOccurrence(request: MoveRequest) {
  const block = request.block
  if (rejectPastDate(block.date)) return
  const targetRoom = request.room_id ?? block.room_id
  const targetStart = request.start_time ?? block.start_time
  const targetEnd = request.end_time ?? block.end_time
  if (!isTimeString(targetStart) || !isTimeString(targetEnd)) {
    toast.error('无法调整本次课程', '时间格式需为 HH:MM')
    return
  }
  if (minutesOf(targetStart) >= minutesOf(targetEnd)) {
    toast.error('无法调整本次课程', '开始时间必须早于结束时间')
    return
  }
  if (
    targetRoom === block.room_id &&
    targetStart === block.start_time &&
    targetEnd === block.end_time
  ) {
    return
  }

  saving.value = true
  try {
    const target = await resolveScheduleForRoom(targetRoom)
    if (!target) throw new Error('目标房间所属课表创建失败')
    assertLoaded(block.schedule_id, target.id)
    assertScheduleCoversDate(target, block.date, '目标房间')
    const plans = planOccurrenceMove(
      payload.value,
      block,
      { room_id: targetRoom, start_time: targetStart, end_time: targetEnd },
      target,
    )
    if (!plans.length) throw new Error('未找到当天对应的课程，请刷新后重试')
    const message = validateExceptionPlans(plans)
    if (message) {
      toast.error('无法调整本次课程', message)
      return
    }
    const targetRoomInfo = payload.value?.rooms.find((room) => room.id === targetRoom) ?? null
    const snapshots = snapshotExceptions(payload.value, exceptionPlanRefs(plans))
    await applyExceptionPlans(plans)
    pushHistory({
      label: '本次调课',
      undo: () => applyExceptionSnapshots(snapshots),
      redo: () => applyExceptionPlans(plans),
    })
    toast.success(
      '已调整本次课程',
      `${formatMonthDay(block.date)} ${targetRoomInfo?.code ?? ''} ${targetStart}–${targetEnd}`,
    )
    await load()
  } catch (caught) {
    toast.error('调整本次课程失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

async function onMove(request: MoveRequest) {
  if (!canManage.value || saving.value) return
  const block = request.block
  if (viewMode.value === 'timeline' && editScope.value === 'occurrence') {
    await moveOccurrence(request)
    return
  }
  if (block.source !== 'template') {
    if (viewMode.value === 'timeline') {
      toast.info(
        '单日覆盖块不能改模板',
        '切换到「本次」作用域调整这一天，或在抽屉点「恢复本次」后再修改周模板。',
      )
    }
    return
  }

  let start = request.start_time
  let end = request.end_time
  if (!start || !end) {
    const span = durationMinutes(block.start_time, block.end_time)
    const period = BOARD_PERIODS[request.period_index ?? 0] ?? BOARD_PERIODS[0]!
    start = period.start
    end = addMinutes(period.start, span > 0 ? span : 100)
  }
  const targetRoom = request.room_id ?? block.room_id
  const targetDay = request.day_of_week ?? block.day_of_week
  if (
    targetRoom === block.room_id &&
    targetDay === block.day_of_week &&
    start === block.start_time &&
    end === block.end_time
  ) {
    return
  }

  const value = moveFormOfBlock(block, {
    room_id: targetRoom,
    day_of_week: targetDay,
    start_time: start,
    end_time: end,
  })
  const message = validateSlotForm(value)
  if (message) {
    toast.error('无法移动课程', message)
    return
  }

  saving.value = true
  try {
    const target = await resolveScheduleForRoom(targetRoom)
    if (!target) throw new Error('目标房间所属课表创建失败')
    assertLoaded(block.schedule_id, target.id)
    const plan = planUpdateSlot(payload.value, block.schedule_id, block.slot_id, target, value)
    const snapshots = snapshotTemplates(payload.value, [block.schedule_id, target.id])
    await applyPlan(plan)
    pushHistory({
      label: '移动课程',
      undo: () => applyPlan(snapshots),
      redo: () => applyPlan(plan),
    })
    toast.success('课程已移动', successDescription(value))
    await load()
  } catch (caught) {
    toast.error('移动课程失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

async function confirmDelete() {
  const block = deleteTarget.value
  if (!block || deleting.value) return
  deleting.value = true
  try {
    assertLoaded(block.schedule_id)
    const plan = planDeleteSlot(payload.value, block.schedule_id, block.slot_id)
    const snapshots = snapshotTemplates(payload.value, [block.schedule_id])
    await applyPlan(plan)
    deleteTarget.value = null
    closeDrawer()
    pushHistory({
      label: '删除课程',
      undo: () => applyPlan(snapshots),
      redo: () => applyPlan(plan),
    })
    toast.success('课程已删除', `${block.course_name} · ${formatMonthDay(block.date)}`)
    await load()
  } catch (caught) {
    toast.error('删除课程失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    deleting.value = false
  }
}

/** 本次停课：只停当天这一节（当天没有其它课时落为整天 cancel）。 */
async function cancelOccurrence() {
  const block = selectedBlock.value
  if (!block || !canManage.value || saving.value) return
  if (rejectPastDate(block.date)) return
  saving.value = true
  try {
    const plans = planOccurrenceCancel(payload.value, block)
    if (!plans.length) throw new Error('未找到当天对应的课程，请刷新后重试')
    const message = validateExceptionPlans(plans)
    if (message) throw new Error(message)
    const snapshots = snapshotExceptions(payload.value, exceptionPlanRefs(plans))
    await applyExceptionPlans(plans)
    closeDrawer()
    pushHistory({
      label: '本次停课',
      undo: () => applyExceptionSnapshots(snapshots),
      redo: () => applyExceptionPlans(plans),
    })
    toast.success(
      '已停课本次',
      `${block.course_name} · ${formatMonthDay(block.date)} ${block.start_time}–${block.end_time}`,
    )
    await load()
  } catch (caught) {
    toast.error('本次停课失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

/** 恢复本次：删除当天单日覆盖，让这一天回到周模板。 */
async function restoreOccurrence() {
  const block = selectedBlock.value
  if (!block || !canManage.value || saving.value) return
  if (rejectPastDate(block.date)) return
  saving.value = true
  try {
    const plans = planOccurrenceRestore(payload.value, block)
    if (!plans.length) throw new Error('当天没有单日覆盖，无需恢复')
    const snapshots = snapshotExceptions(payload.value, exceptionPlanRefs(plans))
    await applyExceptionPlans(plans)
    closeDrawer()
    pushHistory({
      label: '恢复本次',
      undo: () => applyExceptionSnapshots(snapshots),
      redo: () => applyExceptionPlans(plans),
    })
    toast.success('已恢复本次', `${formatMonthDay(block.date)} 已恢复为周模板`)
    await load()
  } catch (caught) {
    toast.error('恢复本次失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

/** 整天停课后画布没有课块可点，用顶部提示条恢复当天所有被停课的课表。 */
async function restoreCancelledDay() {
  if (!canManage.value || saving.value) return
  if (rejectPastDate(selectedDate.value)) return
  const targets = cancelledDaySchedules.value
  if (!targets.length) return
  saving.value = true
  try {
    const plans: ExceptionPlan[] = targets.map((schedule) => ({
      schedule_id: schedule.id,
      date: selectedDate.value,
      action: 'delete' as const,
      slots: [],
    }))
    const snapshots = snapshotExceptions(payload.value, exceptionPlanRefs(plans))
    await applyExceptionPlans(plans)
    pushHistory({
      label: '恢复当天停课',
      undo: () => applyExceptionSnapshots(snapshots),
      redo: () => applyExceptionPlans(plans),
    })
    toast.success('已恢复本次', `${formatMonthDay(selectedDate.value)} 已恢复为周模板`)
    await load()
  } catch (caught) {
    toast.error('恢复本次失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

/** 框选房间缺课表时先 ensure，并重新 load 让批量计划拿到 schedule_id。 */
async function ensureSchedulesForMatrixRooms(roomIds: string[]) {
  const missing = [...new Set(roomIds)].filter((roomId) => !scheduleOfRoom(payload.value, roomId))
  if (!missing.length) return
  const range = defaultScheduleRange(weekStart.value)
  await api.ensureSchedules({
    room_ids: missing,
    start_date: range.start_date,
    end_date: range.end_date,
    name: '默认课表',
  })
  await load()
}

function openBatchFill() {
  if (!canManage.value || saving.value || !matrixSelection.value.length) return
  batchFillError.value = ''
  batchFillOpen.value = true
}

function openBatchClear() {
  if (!canManage.value || saving.value || !matrixSelection.value.length) return
  batchClearOpen.value = true
}

/** 批量应用：ensure 缺课表 → 重新计划 → 按课表全量替换 + rebuild，一次操作只记一条撤销历史。 */
async function runBatchFill(input: BatchFillValue & { mode: BatchFillMode }) {
  if (!canManage.value || saving.value) return
  saving.value = true
  batchFillError.value = ''
  try {
    const cells = [...matrixSelection.value]
    await ensureSchedulesForMatrixRooms(cells.map((cell) => cell.room_id))
    const result = planBatchFill(payload.value, cells, input, input.mode)
    if (result.errors.length) throw new Error(result.errors[0])
    if (result.plans.length) {
      assertLoaded(...result.plans.map((plan) => plan.schedule_id))
      const snapshots = snapshotTemplates(
        payload.value,
        result.plans.map((plan) => plan.schedule_id),
      )
      await applyPlan(result.plans)
      pushHistory({
        label: '批量应用课程',
        undo: () => applyPlan(snapshots),
        redo: () => applyPlan(result.plans),
      })
    }
    batchFillOpen.value = false
    matrixSelection.value = []
    toast.success(`已批量新建 ${result.created} 节（跳过 ${result.skipped}）`)
    await load()
  } catch (caught) {
    batchFillError.value = actionError(caught, '批量应用失败，请稍后重试')
    toast.error('批量应用失败', batchFillError.value)
    await load()
  } finally {
    saving.value = false
  }
}

/** 批量清除：移除选中格同房间同节次的周模板课程；同样只记一条撤销历史。 */
async function runBatchClear() {
  if (!canManage.value || saving.value) return
  saving.value = true
  try {
    const cells = [...matrixSelection.value]
    const result = planBatchClear(payload.value, cells)
    if (result.plans.length) {
      assertLoaded(...result.plans.map((plan) => plan.schedule_id))
      const snapshots = snapshotTemplates(
        payload.value,
        result.plans.map((plan) => plan.schedule_id),
      )
      await applyPlan(result.plans)
      pushHistory({
        label: '批量清除',
        undo: () => applyPlan(snapshots),
        redo: () => applyPlan(result.plans),
      })
    }
    batchClearOpen.value = false
    matrixSelection.value = []
    toast.success(`已清除 ${result.cleared} 节`)
    await load()
  } catch (caught) {
    toast.error('批量清除失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

function sameMatrixCells(left: MatrixCellRef[], right: MatrixCellRef[]): boolean {
  if (left.length !== right.length) return false
  const keys = new Set(
    left.map((cell) => `${cell.room_id}|${cell.day_of_week}|${cell.period_index}`),
  )
  return right.every((cell) => keys.has(`${cell.room_id}|${cell.day_of_week}|${cell.period_index}`))
}

/** 复制当前选区为剪贴板（仅周模板课块；空选区/纯空格子不可复制）。 */
function copyMatrixSelection() {
  if (!canManage.value || saving.value || viewMode.value !== 'matrix') return
  if (!matrixSelection.value.length) return
  const clipboard = buildMatrixClipboard(payload.value, visibleRooms.value, matrixSelection.value)
  if (!clipboard.cells.length) {
    toast.info(
      '所选区域没有可复制的课程',
      'C 矩阵只复制周模板课块；空格子与单日覆盖块不会进入剪贴板。',
    )
    return
  }
  matrixClipboard.value = clipboard
  clipboardSelection.value = matrixSelection.value.map((cell) => ({ ...cell }))
  pasteMode.value = false
  const slotCount = clipboard.cells.reduce((total, cell) => total + cell.slots.length, 0)
  toast.success(
    `已复制 ${slotCount} 节课程`,
    `${clipboard.rows.length * clipboard.columns.length} 格 · ${clipboard.rows.length} 间教室`,
  )
}

function cancelMatrixClipboard() {
  matrixClipboard.value = null
  clipboardSelection.value = null
  pasteMode.value = false
  pendingPasteAnchor.value = null
  pendingPastePreview.value = null
}

function enterPasteMode() {
  if (!canManage.value || saving.value || !matrixClipboard.value) return
  pasteMode.value = true
}

function exitPasteMode() {
  pasteMode.value = false
  pendingPasteAnchor.value = null
  pendingPastePreview.value = null
  pasteConfirmOpen.value = false
}

/** 粘贴模式点击锚点：先算预览（越界/重叠错误直接取消），确认后落库。 */
function onMatrixPaste(anchor: MatrixCellRef) {
  const clipboard = matrixClipboard.value
  if (!pasteMode.value || !clipboard || saving.value || !canManage.value) return
  pendingPasteAnchor.value = anchor
  const preview = planBatchPaste(payload.value, clipboard, anchor, visibleRooms.value)
  if (preview.errors.length) {
    toast.error('无法粘贴', preview.errors[0])
    exitPasteMode()
    return
  }
  if (!preview.created) {
    toast.info('没有可粘贴的课程', '剪贴板中的课程全部超出矩阵范围。')
    exitPasteMode()
    return
  }
  pendingPastePreview.value = preview
  pasteConfirmOpen.value = true
}

/** 确认粘贴：目标房间缺课表先 ensure → 重新计划 → 按课表全量替换 + rebuild，一次只记一条历史。 */
async function runBatchPaste() {
  const anchor = pendingPasteAnchor.value
  const clipboard = matrixClipboard.value
  const preview = pendingPastePreview.value
  if (!anchor || !clipboard || saving.value) return
  saving.value = true
  try {
    const targetRoomIds = preview?.perRoom.map((room) => room.room_id) ?? []
    await ensureSchedulesForMatrixRooms(targetRoomIds)
    const result = planBatchPaste(payload.value, clipboard, anchor, visibleRooms.value)
    if (result.errors.length) throw new Error(result.errors[0])
    if (result.plans.length) {
      assertLoaded(...result.plans.map((plan) => plan.schedule_id))
      const snapshots = snapshotTemplates(
        payload.value,
        result.plans.map((plan) => plan.schedule_id),
      )
      await applyPlan(result.plans)
      pushHistory({
        label: '粘贴课程',
        undo: () => applyPlan(snapshots),
        redo: () => applyPlan(result.plans),
      })
    }
    pasteConfirmOpen.value = false
    pendingPasteAnchor.value = null
    pendingPastePreview.value = null
    matrixClipboard.value = null
    clipboardSelection.value = null
    pasteMode.value = false
    matrixSelection.value = []
    toast.success(`已粘贴 ${result.created} 节课程（覆盖 ${result.overwritten} 节）`)
    await load()
  } catch (caught) {
    toast.error('粘贴课程失败', actionError(caught, '请稍后重试'))
    await load()
  } finally {
    saving.value = false
  }
}

function locateConflict(conflict: BoardConflict) {
  const date = conflict.date
  weekStart.value = weekStartOf(date)
  if (viewMode.value === 'timeline') selectedDate.value = date
  const block = blockData.value.blocks.find((item) => item.key === conflict.block_keys[0]) ?? null
  if (block) {
    selectedKey.value = block.key
    drawerOpen.value = true
  }
}

let syncing = false

function syncQuery() {
  const query: Record<string, string> = { view: viewMode.value, week: weekStart.value }
  if (viewMode.value === 'timeline') query.date = selectedDate.value
  if (areaId.value) query.area = areaId.value
  if (roomId.value) query.room = roomId.value
  const keys = ['view', 'week', 'date', 'area', 'room']
  const same = keys.every((key) => query[key] === queryString(route.query[key]))
  if (same || syncing) return
  syncing = true
  void router.replace({ name: 'schedules', query }).finally(() => {
    syncing = false
  })
}

watch([weekStart, areaId], () => {
  void load()
})

/** 切到时间轴默认「本次」；切回周网格/矩阵固定「模板」。 */
watch(viewMode, (mode) => {
  editScope.value = mode === 'timeline' ? 'occurrence' : 'template'
})

/** 切换视图 / 星期 / 区域时清空框选，避免隐藏的旧选区被批量执行。 */
watch([viewMode, weekStart, areaId], () => {
  matrixSelection.value = []
  matrixClipboard.value = null
  clipboardSelection.value = null
  pasteMode.value = false
  pasteConfirmOpen.value = false
  pendingPasteAnchor.value = null
  pendingPastePreview.value = null
  batchFillOpen.value = false
  batchClearOpen.value = false
})

/** 选区变化后旧剪贴板失效（避免把 A 区复制的内容粘到 B 区的相对矩形上）。 */
watch(matrixSelection, (cells) => {
  const snapshot = clipboardSelection.value
  if (!matrixClipboard.value || !snapshot) return
  if (sameMatrixCells(cells, snapshot)) return
  matrixClipboard.value = null
  clipboardSelection.value = null
  pasteMode.value = false
})

watch([viewMode, weekStart, selectedDate, areaId, roomId], syncQuery)

watch(selectedBlock, (block) => {
  if (!block) drawerOpen.value = false
})

onMounted(() => {
  window.addEventListener('keydown', onHistoryKeydown, true)
  void load()
})

onUnmounted(() => {
  window.removeEventListener('keydown', onHistoryKeydown, true)
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col px-7 py-5">
    <div class="mx-auto flex min-h-0 w-full max-w-[1440px] flex-1 flex-col">
      <PageHeader
        title="排课工作台"
        :description="headerDescription"
        :badge="canManage ? '' : '只读'"
      />

      <div
        v-if="error"
        class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2 text-[11.5px] text-rose-700"
      >
        <span class="flex items-center gap-1.5">
          <AppIcon name="alert" :size="13" />
          {{ error }}
        </span>
        <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
      </div>

      <section
        class="mt-3 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-line bg-surface"
      >
        <BoardToolbar
          v-model:view-mode="viewMode"
          v-model:area-id="areaId"
          v-model:room-id="roomId"
          v-model:keyword="keyword"
          v-model:only-conflicts="onlyConflicts"
          v-model:scope="editScope"
          :week-start="weekStart"
          :selected-date="selectedDate"
          :areas="payload?.areas ?? []"
          :rooms="areaRooms"
          :conflicts="visibleConflicts"
          :loading="loading"
          :can-manage="canManage"
          :can-undo="canUndo"
          :can-redo="canRedo"
          :undo-label="undoLabel"
          :redo-label="redoLabel"
          @prev-week="shiftWeek(-1)"
          @next-week="shiftWeek(1)"
          @prev-day="shiftDay(-1)"
          @next-day="shiftDay(1)"
          @today="goToday"
          @refresh="load()"
          @create="onCreateFromToolbar"
          @undo="undo"
          @redo="redo"
          @locate="locateConflict"
        />

        <div
          v-if="emptyHint"
          class="flex items-center justify-between gap-3 border-b border-line bg-canvas/60 px-3 py-1.5 text-[10.5px] text-muted"
        >
          <span class="flex min-w-0 items-center gap-1.5">
            <AppIcon name="info" :size="12" class="shrink-0" />
            <b class="shrink-0 font-medium text-ink-soft">{{ emptyHint.title }}</b>
            <span class="truncate">{{ emptyHint.description }}</span>
          </span>
          <AppButton
            v-if="emptyHint.action === 'create'"
            size="sm"
            icon="plus"
            variant="secondary"
            @click="onCreateFromToolbar"
          >
            新建课程
          </AppButton>
          <AppButton
            v-else-if="emptyHint.action === 'reset'"
            size="sm"
            icon="filter"
            variant="secondary"
            @click="resetFilters"
          >
            清除筛选
          </AppButton>
        </div>

        <div
          v-if="viewMode === 'timeline' && cancelledDaySchedules.length"
          class="flex items-center justify-between gap-3 border-b border-line bg-amber-50/70 px-3 py-1.5 text-[10.5px] text-amber-800"
        >
          <span class="flex min-w-0 items-center gap-1.5">
            <AppIcon name="calendar" :size="12" class="shrink-0" />
            <b class="shrink-0 font-medium">当天停课</b>
            <span class="truncate"> {{ cancelledDayLabel }} 这一天已停课，可恢复为周模板。 </span>
          </span>
          <AppButton
            v-if="canManage"
            size="sm"
            variant="secondary"
            icon="rotate-ccw"
            :disabled="saving"
            @click="restoreCancelledDay"
          >
            恢复本次
          </AppButton>
        </div>

        <template v-if="viewMode === 'matrix'">
          <!-- 粘贴模式 -->
          <div
            v-if="pasteMode && matrixClipboard"
            class="flex flex-wrap items-center justify-between gap-2 border-b border-accent/40 bg-accent-soft/70 px-3 py-1.5 text-[10.5px] text-ink-soft"
          >
            <span class="flex min-w-0 items-center gap-1.5">
              <AppIcon name="copy" :size="12" class="shrink-0 text-accent" />
              <b class="font-medium">粘贴模式</b>
              <span class="truncate text-muted">点击目标格粘贴（Esc 取消）</span>
            </span>
            <span class="flex shrink-0 items-center gap-1.5">
              <AppButton size="sm" variant="ghost" icon="x" @click="exitPasteMode">
                取消粘贴
              </AppButton>
            </span>
          </div>

          <!-- 粘贴就绪（已复制） -->
          <div
            v-else-if="matrixClipboard"
            class="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-accent-soft/50 px-3 py-1.5 text-[10.5px] text-ink-soft"
          >
            <span class="flex min-w-0 items-center gap-1.5">
              <AppIcon name="copy" :size="12" class="shrink-0 text-accent" />
              <b class="font-medium">已复制 {{ clipboardCellCount }} 格</b>
              <span class="shrink-0 text-muted">（跨 {{ clipboardRoomCount }} 间教室）</span>
              <span class="truncate text-faint">· 选择目标后按相对位置粘贴，Esc 取消</span>
            </span>
            <span class="flex shrink-0 items-center gap-1.5">
              <AppButton
                size="sm"
                variant="primary"
                icon="arrow-right"
                :disabled="saving"
                @click="enterPasteMode"
              >
                粘贴
              </AppButton>
              <AppButton
                size="sm"
                variant="ghost"
                icon="x"
                :disabled="saving"
                @click="cancelMatrixClipboard"
              >
                取消复制
              </AppButton>
            </span>
          </div>

          <!-- 框选操作 -->
          <div
            v-else-if="matrixSelection.length"
            class="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-accent-soft/50 px-3 py-1.5 text-[10.5px] text-ink-soft"
          >
            <span class="flex min-w-0 items-center gap-1.5">
              <AppIcon name="overview" :size="12" class="shrink-0 text-accent" />
              <b class="font-medium">已选 {{ matrixSelection.length }} 格</b>
              <span class="shrink-0 text-muted">（跨 {{ matrixSelectedRoomCount }} 间教室）</span>
              <span class="truncate text-faint">· 拖拽调整选区，Esc 取消</span>
            </span>
            <span class="flex shrink-0 items-center gap-1.5">
              <AppButton
                size="sm"
                variant="primary"
                icon="plus"
                :disabled="saving"
                @click="openBatchFill"
              >
                批量应用课程
              </AppButton>
              <AppButton
                size="sm"
                variant="danger"
                icon="trash"
                :disabled="saving"
                @click="openBatchClear"
              >
                批量清除
              </AppButton>
              <AppButton
                size="sm"
                variant="secondary"
                icon="copy"
                :disabled="saving || !matrixCopyable"
                :title="
                  matrixCopyable ? '复制所选周模板课块（Ctrl+C）' : '所选区域没有可复制的周模板课程'
                "
                @click="copyMatrixSelection"
              >
                复制
              </AppButton>
              <AppButton
                size="sm"
                variant="ghost"
                icon="x"
                :disabled="saving"
                @click="matrixSelection = []"
              >
                取消选择
              </AppButton>
            </span>
          </div>
        </template>

        <div class="relative min-h-0 flex-1">
          <div v-if="loading && !payload" class="absolute inset-0 flex flex-col gap-2 p-4">
            <div class="flex shrink-0 gap-2">
              <div
                v-for="index in 7"
                :key="`sk-head-${index}`"
                class="h-7 flex-1 animate-pulse rounded bg-line/60"
              />
            </div>
            <div v-for="row in 5" :key="`sk-row-${row}`" class="flex min-h-0 flex-1 gap-2">
              <div class="w-[76px] h-full animate-pulse rounded bg-line/60" />
              <div
                v-for="index in 7"
                :key="`sk-cell-${row}-${index}`"
                class="h-full flex-1 animate-pulse rounded bg-line/60"
              />
            </div>
          </div>

          <div
            v-else-if="error && !payload"
            class="absolute inset-0 flex items-center justify-center"
          >
            <EmptyState title="排课数据加载失败" :description="error" icon="alert">
              <template #action>
                <AppButton size="sm" icon="refresh" variant="primary" @click="load()">
                  重新加载
                </AppButton>
              </template>
            </EmptyState>
          </div>

          <template v-else-if="payload">
            <div
              v-if="!payload.rooms.length"
              class="absolute inset-0 flex items-center justify-center"
            >
              <EmptyState
                title="还没有可用房间"
                description="排课工作台基于房间的课表运作；请先在「房间」页面创建并启用房间。"
                icon="door"
              />
            </div>

            <template v-else>
              <WeekGridView
                v-if="viewMode === 'week'"
                class="h-full"
                :week-start="weekStart"
                :rooms="visibleRooms"
                :blocks="visibleBlocks"
                :selected-key="selectedKey"
                :can-manage="dragEnabled"
                @select="onSelect"
                @create="onCreate"
                @move="onMove"
              />
              <ResourceTimelineView
                v-else-if="viewMode === 'timeline'"
                class="h-full"
                :date="selectedDate"
                :rooms="visibleRooms"
                :blocks="visibleBlocks"
                :selected-key="selectedKey"
                :can-manage="dragEnabled"
                @select="onSelect"
                @create="onCreate"
                @move="onMove"
              />
              <AllRoomMatrixView
                v-else
                v-model:selection="matrixSelection"
                class="h-full"
                :week-start="weekStart"
                :rooms="visibleRooms"
                :blocks="visibleBlocks"
                :selected-key="selectedKey"
                :can-manage="dragEnabled"
                :pasting="pasteMode"
                :paste-rows="matrixClipboard?.rows.length ?? 0"
                :paste-cols="matrixClipboard?.columns.length ?? 0"
                @select="onSelect"
                @create="onCreate"
                @move="onMove"
                @paste="onMatrixPaste"
              />
            </template>

            <BlockDrawer
              :open="drawerOpen"
              :block="selectedBlock"
              :rooms="payload.rooms"
              :can-manage="canManage"
              :deleting="deleting || saving"
              :scope="editScope"
              :can-adjust-occurrence="canAdjustOccurrence"
              @close="closeDrawer"
              @edit="openEdit"
              @delete="requestDelete"
              @cancel-occurrence="cancelOccurrence"
              @restore-occurrence="restoreOccurrence"
            />
          </template>
        </div>
      </section>
    </div>

    <BlockEditModal
      :open="editorOpen"
      :mode="editorMode"
      :rooms="payload?.rooms ?? []"
      :initial="editorInitial"
      :scope="editorMode === 'create' ? (createScene?.scope ?? 'template') : 'template'"
      :scope-date="createScene?.date ?? ''"
      :saving="saving"
      :error="editorError"
      @close="editorOpen = false"
      @submit="submitEditor"
    />

    <ConfirmDialog
      :open="pasteConfirmOpen"
      title="粘贴课程"
      :description="
        pendingPastePreview
          ? `将粘贴 ${pendingPastePreview.created} 节课程，覆盖目标格 ${pendingPastePreview.overwritten} 节课程，涉及 ${pendingPastePreview.perRoom.length} 张课表。`
          : ''
      "
      detail="按剪贴板相对矩形写入周模板：源格有课的目标格会先移除同节次现有课程再写入，源格为空的目标格保持不动。操作会立即保存并重建未来实例，可用 Ctrl+Z 撤销。"
      confirm-label="粘贴"
      :loading="saving"
      @confirm="runBatchPaste"
      @close="exitPasteMode"
    />

    <BatchFillModal
      :open="batchFillOpen"
      :payload="payload"
      :cells="matrixSelection"
      :saving="saving"
      :error="batchFillError"
      @close="batchFillOpen = false"
      @submit="runBatchFill"
    />

    <ConfirmDialog
      :open="batchClearOpen"
      title="批量清除课程"
      :description="`清除所选 ${matrixSelection.length} 格（跨 ${matrixSelectedRoomCount} 间教室）内的周模板课程。`"
      detail="只会移除这些格子对应房间、同节次的全部周模板课程；其它课程与单日覆盖不受影响，操作会立即保存并重建未来实例。"
      confirm-label="批量清除"
      danger
      :loading="saving"
      @confirm="runBatchClear"
      @close="batchClearOpen = false"
    />

    <ConfirmDialog
      :open="!!deleteTarget"
      title="删除课程"
      :description="
        deleteTarget
          ? `从周模板中删除「${deleteTarget.course_name}」（${weekdayName(deleteTarget.day_of_week)} ${deleteTarget.start_time}–${deleteTarget.end_time}）。`
          : ''
      "
      detail="删除后该时段不再生成排课实例；其它课块不受影响，操作会立即保存并重建未来实例。"
      confirm-label="删除课程"
      danger
      :loading="deleting"
      @confirm="confirmDelete"
      @close="deleteTarget = null"
    />
  </div>
</template>
