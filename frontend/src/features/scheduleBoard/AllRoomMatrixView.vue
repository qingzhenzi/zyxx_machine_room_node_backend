<script setup lang="ts">
/**
 * C · 全房间编排矩阵（模板周）。
 *
 * 行 = 房间（左侧粘性列），列 = 周一~周日 × BOARD_PERIODS（双层粘性表头，横向滚动）。
 * 纯展示组件：数据全部来自 props，点空格新建 / 拖拽课块换格 / 选中 都只用 emits 抛出。
 *
 * 课块归属节次用 `periodPlacement(block.start_time).index`（跨节次块按起始节次放）；
 * 同一格多块时纵向堆叠，超过 2 块折叠为「+N 节」（简单处理）。
 * 拖动用原生 HTML5 DnD：目标格高亮，落格后 emit move（period_index 换算由父级负责）。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import EmptyState from '@/components/ui/EmptyState.vue'

import {
  BOARD_PERIODS,
  formatMonthDay,
  periodPlacement,
  sortRooms,
  weekDates,
  weekdayName,
  type BoardBlock,
  type BoardPeriod,
  type BoardRoom,
} from './boardTypes'
import ScheduleBlockChip from './ScheduleBlockChip.vue'
import type { MatrixCellRef } from './boardActions'

interface MatrixCreatePayload {
  room_id: string
  date: string
  period_index: number
}

interface MatrixMovePayload {
  block: BoardBlock
  room_id: string
  day_of_week: number
  period_index: number
}

interface MatrixDay {
  date: string
  dayOfWeek: number
  label: string
  monthDay: string
  weekend: boolean
}

interface MatrixColumn {
  key: string
  dayIndex: number
  dayOfWeek: number
  date: string
  periodIndex: number
  period: BoardPeriod
  weekend: boolean
  dayStart: boolean
}

const ROOM_COLUMN_WIDTH = 172
const CELL_WIDTH = 88
const ROW_HEIGHT = 64
const VISIBLE_CELL_BLOCKS = 2

const props = defineProps<{
  weekStart: string
  rooms: BoardRoom[]
  blocks: BoardBlock[]
  selectedKey: string | null
  canManage: boolean
  /** 粘贴模式：格子点击作为粘贴锚点，hover 显示目标矩形虚线预览。 */
  pasting?: boolean
  pasteRows?: number
  pasteCols?: number
}>()

const emit = defineEmits<{
  select: [block: BoardBlock]
  create: [payload: MatrixCreatePayload]
  move: [payload: MatrixMovePayload]
  paste: [anchor: MatrixCellRef]
}>()

/** 框选选区（由页面持有，用于批量应用/清除；只读账号不产生选区）。 */
const selection = defineModel<MatrixCellRef[]>('selection', { required: true })

const scrollEl = ref<HTMLElement | null>(null)
const dragBlock = ref<BoardBlock | null>(null)
const dropTarget = ref<string | null>(null)

const draggingKey = computed(() => dragBlock.value?.key ?? '')

const dates = computed(() => weekDates(props.weekStart))

const days = computed<MatrixDay[]>(() =>
  dates.value.map((date, index) => ({
    date,
    dayOfWeek: index + 1,
    label: weekdayName(index + 1),
    monthDay: formatMonthDay(date),
    weekend: index >= 5,
  })),
)

const columns = computed<MatrixColumn[]>(() => {
  const result: MatrixColumn[] = []
  for (const [dayIndex, day] of days.value.entries()) {
    for (const [periodIndex, period] of BOARD_PERIODS.entries()) {
      result.push({
        key: `${day.date}|${period.key}`,
        dayIndex,
        dayOfWeek: day.dayOfWeek,
        date: day.date,
        periodIndex,
        period,
        weekend: day.weekend,
        dayStart: periodIndex === 0,
      })
    }
  }
  return result
})

const sortedRooms = computed(() => sortRooms(props.rooms))

const roomIds = computed(() => new Set(props.rooms.map((room) => room.id)))

interface SelectionAnchor {
  roomIndex: number
  columnIndex: number
}

const selectionDragging = ref(false)
/** 拖拽过程中的本地预览选区：松开后才提交给页面，避免操作条出现导致画布位移。 */
const draftSelection = ref<MatrixCellRef[]>([])
const renderedSelection = computed(() =>
  selectionDragging.value ? draftSelection.value : selection.value,
)

const selectedCellKeys = computed(
  () =>
    new Set(
      renderedSelection.value.map(
        (cell) => `${cell.room_id}|${cell.day_of_week}|${cell.period_index}`,
      ),
    ),
)

function isCellSelected(roomId: string, column: MatrixColumn): boolean {
  return selectedCellKeys.value.has(`${roomId}|${column.dayOfWeek}|${column.periodIndex}`)
}

const pasteHover = ref<SelectionAnchor | null>(null)

function coordsOfCell(
  roomId: string,
  dayOfWeek: number,
  periodIndex: number,
): SelectionAnchor | null {
  const roomIndex = sortedRooms.value.findIndex((room) => room.id === roomId)
  const columnIndex = columns.value.findIndex(
    (column) => column.dayOfWeek === dayOfWeek && column.periodIndex === periodIndex,
  )
  if (roomIndex < 0 || columnIndex < 0) return null
  return { roomIndex, columnIndex }
}

function onCellMouseEnter(roomId: string, dayOfWeek: number, periodIndex: number) {
  if (!props.pasting) return
  pasteHover.value = coordsOfCell(roomId, dayOfWeek, periodIndex)
}

function onPasteGridMouseLeave() {
  pasteHover.value = null
}

/** 粘贴模式下 hover 的目标矩形（按剪贴板行×列，超出矩阵部分自然裁剪）。 */
function isPastePreviewCell(roomId: string, column: MatrixColumn): boolean {
  if (!props.pasting || !pasteHover.value) return false
  const rowCount = Math.max(0, props.pasteRows ?? 0)
  const columnCount = Math.max(0, props.pasteCols ?? 0)
  if (!rowCount || !columnCount) return false
  const coords = coordsOfCell(roomId, column.dayOfWeek, column.periodIndex)
  if (!coords) return false
  return (
    coords.roomIndex >= pasteHover.value.roomIndex &&
    coords.roomIndex < pasteHover.value.roomIndex + rowCount &&
    coords.columnIndex >= pasteHover.value.columnIndex &&
    coords.columnIndex < pasteHover.value.columnIndex + columnCount
  )
}

function rowCellRefs(roomId: string): MatrixCellRef[] {
  return columns.value.map((column) => ({
    room_id: roomId,
    day_of_week: column.dayOfWeek,
    period_index: column.periodIndex,
  }))
}

function isRoomRowSelected(roomId: string): boolean {
  const refs = rowCellRefs(roomId)
  return (
    refs.length > 0 &&
    refs.every((ref) =>
      selectedCellKeys.value.has(`${ref.room_id}|${ref.day_of_week}|${ref.period_index}`),
    )
  )
}

/** 点击房间行头：选中/取消该房间整行（整周 7 天 × 5 节次 = 35 格），方便复制整周。 */
function onRoomHeaderClick(room: BoardRoom) {
  if (!props.canManage || props.pasting) return
  selection.value = isRoomRowSelected(room.id) ? [] : rowCellRefs(room.id)
}

const selectionAnchor = ref<SelectionAnchor | null>(null)
let selectionStartX = 0
let selectionStartY = 0
let suppressClickUntil = 0

function cellCoordsFromElement(element: Element | null): SelectionAnchor | null {
  if (!element) return null
  const cell = element.closest<HTMLElement>('[data-room-id][data-day][data-period]')
  if (!cell) return null
  const roomId = cell.dataset.roomId
  const day = Number(cell.dataset.day)
  const period = Number(cell.dataset.period)
  const roomIndex = sortedRooms.value.findIndex((room) => room.id === roomId)
  const columnIndex = columns.value.findIndex(
    (column) => column.dayOfWeek === day && column.periodIndex === period,
  )
  if (roomIndex < 0 || columnIndex < 0) return null
  return { roomIndex, columnIndex }
}

/** 以锚点与当前格为对角，生成矩形选区（跨房间行 / 跨天、跨节次列）。 */
function applySelectionRect(anchor: SelectionAnchor, current: SelectionAnchor) {
  const roomStart = Math.min(anchor.roomIndex, current.roomIndex)
  const roomEnd = Math.max(anchor.roomIndex, current.roomIndex)
  const columnStart = Math.min(anchor.columnIndex, current.columnIndex)
  const columnEnd = Math.max(anchor.columnIndex, current.columnIndex)
  const refs: MatrixCellRef[] = []
  for (let roomIndex = roomStart; roomIndex <= roomEnd; roomIndex += 1) {
    const room = sortedRooms.value[roomIndex]
    if (!room) continue
    for (let columnIndex = columnStart; columnIndex <= columnEnd; columnIndex += 1) {
      const column = columns.value[columnIndex]
      if (!column) continue
      refs.push({
        room_id: room.id,
        day_of_week: column.dayOfWeek,
        period_index: column.periodIndex,
      })
    }
  }
  draftSelection.value = refs
}

function cancelSelectionGesture() {
  selectionAnchor.value = null
  selectionDragging.value = false
  draftSelection.value = []
  window.removeEventListener('mousemove', onSelectionMouseMove)
  window.removeEventListener('mouseup', onSelectionMouseUp)
}

function onSelectionMouseMove(event: MouseEvent) {
  const anchor = selectionAnchor.value
  if (!anchor) return
  if (!selectionDragging.value) {
    const deltaX = event.clientX - selectionStartX
    const deltaY = event.clientY - selectionStartY
    if (deltaX * deltaX + deltaY * deltaY < 16) return
    selectionDragging.value = true
  }
  const current = cellCoordsFromElement(document.elementFromPoint(event.clientX, event.clientY))
  if (!current) return
  applySelectionRect(anchor, current)
}

function onSelectionMouseUp() {
  const dragged = selectionDragging.value
  const draft = draftSelection.value
  cancelSelectionGesture()
  if (dragged) {
    // 拖拽结束后浏览器仍会派发 click，短暂抑制以免误触「新建」。
    suppressClickUntil = Date.now() + 250
    selection.value = draft
  } else {
    selection.value = []
  }
}

/** 在矩阵格子上按下开始框选；课块上按下不拦截（保留课块拖拽）。 */
function onCellMouseDown(event: MouseEvent) {
  if (!props.canManage || props.pasting || event.button !== 0) return
  const target = event.target as HTMLElement | null
  if (target?.closest('[data-block-key]')) return
  const anchor = cellCoordsFromElement(event.currentTarget as Element | null)
  if (!anchor) return
  event.preventDefault()
  cancelSelectionGesture()
  selectionAnchor.value = anchor
  selectionDragging.value = false
  selectionStartX = event.clientX
  selectionStartY = event.clientY
  window.addEventListener('mousemove', onSelectionMouseMove)
  window.addEventListener('mouseup', onSelectionMouseUp)
}

function onSelectionKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || props.pasting) return
  if (!selection.value.length && !selectionAnchor.value) return
  cancelSelectionGesture()
  selection.value = []
}

/** 只展示本周、当前筛选房间内的课块（父级可能传全周，重复过滤安全）。 */
const weekBlocks = computed(() => {
  const dateSet = new Set(dates.value)
  return props.blocks.filter((block) => dateSet.has(block.date) && roomIds.value.has(block.room_id))
})

function cellKeyOf(roomId: string, dayIndex: number, periodIndex: number): string {
  return `${roomId}|${dayIndex}|${periodIndex}`
}

const blocksByCell = computed(() => {
  const result = new Map<string, BoardBlock[]>()
  const dayIndexByDate = new Map(dates.value.map((date, index) => [date, index]))
  for (const block of weekBlocks.value) {
    const dayIndex = dayIndexByDate.get(block.date)
    if (dayIndex === undefined) continue
    const { index } = periodPlacement(block.start_time)
    const key = cellKeyOf(block.room_id, dayIndex, index)
    const list = result.get(key)
    if (list) list.push(block)
    else result.set(key, [block])
  }
  for (const list of result.values()) {
    list.sort((left, right) => left.start_time.localeCompare(right.start_time))
  }
  return result
})

function cellBlocksOf(roomId: string, dayIndex: number, periodIndex: number): BoardBlock[] {
  return blocksByCell.value.get(cellKeyOf(roomId, dayIndex, periodIndex)) ?? []
}

function visibleCellBlocks(roomId: string, dayIndex: number, periodIndex: number): BoardBlock[] {
  return cellBlocksOf(roomId, dayIndex, periodIndex).slice(0, VISIBLE_CELL_BLOCKS)
}

function cellExtraCount(roomId: string, dayIndex: number, periodIndex: number): number {
  const total = cellBlocksOf(roomId, dayIndex, periodIndex).length
  return total > VISIBLE_CELL_BLOCKS ? total - VISIBLE_CELL_BLOCKS : 0
}

function isCellDense(roomId: string, dayIndex: number, periodIndex: number): boolean {
  return cellBlocksOf(roomId, dayIndex, periodIndex).length >= 2
}

function cellClass(roomId: string, column: MatrixColumn): string[] {
  const classes: string[] = ['mx-cell']
  const isDropTarget = dropTarget.value === cellKeyOf(roomId, column.dayIndex, column.periodIndex)
  if (isDropTarget) {
    classes.push('bg-accent-soft/80', 'ring-1', 'ring-accent/50', 'ring-inset')
  } else if (column.weekend) {
    classes.push('bg-[#fcfcfd]')
  } else {
    classes.push('bg-surface')
  }
  if (column.dayStart) classes.push('border-l', 'border-line-strong')
  if (props.pasting) classes.push('cursor-copy')
  else if (props.canManage) classes.push('cursor-crosshair', 'hover:bg-[#f6f8fc]')
  if (isCellDense(roomId, column.dayIndex, column.periodIndex)) classes.push('is-dense')
  return classes
}

function blockTitle(block: BoardBlock): string {
  const teacher = block.teacher ? ` · ${block.teacher}` : ''
  const source = block.source === 'exception' ? ' · 单日覆盖' : ''
  const conflict = block.conflicts.some((item) => item.kind === 'room')
    ? ' · 房间冲突'
    : block.conflicts.some((item) => item.kind === 'teacher')
      ? ' · 教师冲突'
      : ''
  return `${block.course_name}${teacher} · ${block.room_code} · ${block.start_time}–${block.end_time}${source}${conflict}`
}

function deviceState(room: BoardRoom): { className: string; label: string } {
  if (!room.device) return { className: 'bg-line-strong', label: '未绑定设备' }
  if (room.device.online === true) return { className: 'bg-emerald-500', label: '设备在线' }
  if (room.device.online === false) return { className: 'bg-rose-400', label: '设备离线' }
  return { className: 'bg-faint', label: '设备在线状态未知' }
}

function resetScroll(): void {
  const el = scrollEl.value
  if (!el) return
  el.scrollLeft = 0
  el.scrollTop = 0
}

onMounted(() => {
  resetScroll()
  window.addEventListener('keydown', onSelectionKeydown)
})

onUnmounted(() => {
  cancelSelectionGesture()
  window.removeEventListener('keydown', onSelectionKeydown)
})

watch(
  () => props.weekStart,
  () => {
    cancelSelectionGesture()
    selection.value = []
    void nextTick(resetScroll)
  },
)

watch(
  () => props.pasting,
  (pasting) => {
    if (!pasting) pasteHover.value = null
  },
)

/** 粘贴模式下点课块也算选择粘贴锚点；普通模式点课块打开详情。 */
function onChipClick(block: BoardBlock) {
  if (props.pasting) {
    const periodIndex = periodPlacement(block.start_time).index
    emit('paste', {
      room_id: block.room_id,
      day_of_week: block.day_of_week,
      period_index: periodIndex,
    })
    return
  }
  emit('select', block)
}

function onChipDragStart(block: BoardBlock, event: DragEvent): void {
  if (!props.canManage || props.pasting) {
    event.preventDefault()
    return
  }
  dragBlock.value = block
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', block.key)
  }
}

function onChipDragEnd(): void {
  dragBlock.value = null
  dropTarget.value = null
}

function onCellDragOver(
  room: BoardRoom,
  dayIndex: number,
  periodIndex: number,
  event: DragEvent,
): void {
  if (!props.canManage || !dragBlock.value) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
  dropTarget.value = cellKeyOf(room.id, dayIndex, periodIndex)
}

function onCellDragLeave(event: DragEvent): void {
  const cell = event.currentTarget as HTMLElement | null
  const related = event.relatedTarget as Node | null
  if (cell && related && cell.contains(related)) return
  dropTarget.value = null
}

function onCellDrop(
  room: BoardRoom,
  dayIndex: number,
  periodIndex: number,
  event: DragEvent,
): void {
  event.preventDefault()
  if (!props.canManage) return
  const block = dragBlock.value
  dragBlock.value = null
  dropTarget.value = null
  if (!block) return
  const ownDayIndex = dates.value.indexOf(block.date)
  const ownPeriodIndex = periodPlacement(block.start_time).index
  if (block.room_id === room.id && ownDayIndex === dayIndex && ownPeriodIndex === periodIndex) {
    return
  }
  emit('move', {
    block,
    room_id: room.id,
    day_of_week: dayIndex + 1,
    period_index: periodIndex,
  })
}

/** 点击空格新建；已占用的格子点击不触发（避免在已有课程上弹出新建）。 */
function onCellClick(room: BoardRoom, dayIndex: number, periodIndex: number): void {
  if (!props.canManage || dragBlock.value) return
  if (props.pasting) {
    emit('paste', {
      room_id: room.id,
      day_of_week: dayIndex + 1,
      period_index: periodIndex,
    })
    return
  }
  if (Date.now() < suppressClickUntil) return
  if (cellBlocksOf(room.id, dayIndex, periodIndex).length > 0) return
  const date = dates.value[dayIndex]
  if (!date) return
  emit('create', { room_id: room.id, date, period_index: periodIndex })
}
</script>

<template>
  <div class="flex h-full min-h-[420px] min-w-0 flex-col overflow-hidden bg-surface">
    <EmptyState
      v-if="sortedRooms.length === 0"
      class="my-auto"
      title="暂无可排课房间"
      description="当前筛选条件下没有房间。清除区域或房间筛选后，这里会按房间列出整周编排矩阵。"
      icon="calendar"
    />

    <template v-else>
      <!-- 无课提示（有房间无课，避免白板） -->
      <div
        v-if="weekBlocks.length === 0 && props.blocks.length > 0"
        class="flex h-7 shrink-0 items-center gap-2 border-b border-line bg-accent-soft/40 px-3.5 text-[11px] text-muted"
      >
        <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"></span>
        <span class="truncate">
          本周还没有课程{{
            props.canManage ? '：点击矩阵空格即可新建；拖拽课块可跨房间、跨天移动。' : '。'
          }}
        </span>
      </div>

      <div
        ref="scrollEl"
        class="min-h-0 min-w-0 flex-1 overflow-auto"
        @mouseleave="onPasteGridMouseLeave"
      >
        <div class="w-max">
          <!-- 双层粘性表头（日期 + 节次） -->
          <div class="sticky top-0 z-30 flex w-max border-b border-line bg-[#fbfcfd]">
            <!-- 左上角交叉格（横向粘性） -->
            <div
              class="sticky left-0 z-40 flex shrink-0 flex-col justify-center gap-0.5 border-r border-line-strong bg-[#fbfcfd] px-3"
              :style="{ width: `${ROOM_COLUMN_WIDTH}px` }"
            >
              <span class="text-[10.5px] font-medium text-ink-soft">房间 / 时段</span>
              <span class="truncate text-[9px] text-faint">
                周一–周日 · {{ BOARD_PERIODS.length }} 个节次
              </span>
            </div>

            <div class="flex min-w-0 flex-col">
              <!-- 第一层：日期 -->
              <div class="flex h-7">
                <div
                  v-for="day in days"
                  :key="`day-${day.date}`"
                  class="flex shrink-0 items-center gap-1.5 border-r border-b border-line px-2.5"
                  :class="day.weekend ? 'bg-[#f8f9fb] text-muted' : 'bg-[#fbfcfd] text-ink-soft'"
                  :style="{ width: `${CELL_WIDTH * BOARD_PERIODS.length}px` }"
                >
                  <span class="text-[11px] font-semibold whitespace-nowrap">{{ day.label }}</span>
                  <span class="font-mono text-[9.5px] font-normal text-faint">
                    {{ day.monthDay }}
                  </span>
                </div>
              </div>

              <!-- 第二层：节次 -->
              <div class="flex h-[30px]">
                <div
                  v-for="column in columns"
                  :key="`head-${column.key}`"
                  class="flex shrink-0 flex-col items-center justify-center border-r border-b border-line/70 leading-none"
                  :class="[
                    column.weekend ? 'bg-[#f8f9fb]' : 'bg-[#fbfcfd]',
                    column.dayStart ? 'border-l border-line-strong' : '',
                  ]"
                  :style="{ width: `${CELL_WIDTH}px` }"
                >
                  <span class="text-[9.5px] font-medium whitespace-nowrap text-ink-soft">
                    {{ column.period.label }}
                  </span>
                  <span class="mt-0.5 font-mono text-[8.5px] text-faint">
                    {{ column.period.start }}
                  </span>
                </div>
              </div>
            </div>
          </div>

          <!-- 房间行 -->
          <div
            v-for="room in sortedRooms"
            :key="room.id"
            class="flex w-max"
            :style="{ height: `${ROW_HEIGHT}px` }"
          >
            <div
              class="sticky left-0 z-20 flex shrink-0 flex-col justify-center gap-0.5 border-r border-b border-line-strong bg-surface px-3 transition-colors"
              :class="[
                props.canManage && !props.pasting ? 'cursor-pointer hover:bg-accent-soft/30' : '',
                isRoomRowSelected(room.id) ? 'bg-accent-soft/50' : '',
              ]"
              :data-room-header="room.id"
              :data-room-id="room.id"
              :title="
                props.canManage && !props.pasting
                  ? '点击选中/取消该房间整行（整周 7 天 × 5 节次），方便复制整周'
                  : room.code
              "
              :style="{ width: `${ROOM_COLUMN_WIDTH}px` }"
              @click="onRoomHeaderClick(room)"
            >
              <div class="flex min-w-0 items-baseline gap-1.5">
                <span
                  class="min-w-0 truncate font-mono text-[11px] leading-4 font-semibold text-ink"
                  :title="room.code"
                >
                  {{ room.code }}
                </span>
                <span class="shrink-0 text-[9px] text-faint">{{ room.capacity }} 座</span>
                <span
                  class="inline-block h-1.5 w-1.5 shrink-0 rounded-full"
                  :class="deviceState(room).className"
                  :title="deviceState(room).label"
                ></span>
              </div>
              <span class="truncate text-[9.5px] leading-3 text-muted" :title="room.name">
                {{ room.name }}
              </span>
            </div>

            <div
              v-for="column in columns"
              :key="`${room.id}-${column.key}`"
              class="relative flex shrink-0 flex-col gap-[2px] overflow-hidden border-r border-b border-line/60 p-[3px] transition-colors"
              :data-room-id="room.id"
              :data-day="column.dayOfWeek"
              :data-period="column.periodIndex"
              :class="cellClass(room.id, column)"
              :style="{ width: `${CELL_WIDTH}px` }"
              @mousedown="onCellMouseDown"
              @mouseenter="onCellMouseEnter(room.id, column.dayOfWeek, column.periodIndex)"
              @click="onCellClick(room, column.dayIndex, column.periodIndex)"
              @dragover="onCellDragOver(room, column.dayIndex, column.periodIndex, $event)"
              @dragleave="onCellDragLeave"
              @drop="onCellDrop(room, column.dayIndex, column.periodIndex, $event)"
            >
              <ScheduleBlockChip
                v-for="block in visibleCellBlocks(room.id, column.dayIndex, column.periodIndex)"
                :key="block.key"
                :data-block-key="block.key"
                class="shrink-0"
                :block="block"
                compact
                :show-room="false"
                :selected="block.key === props.selectedKey"
                :dragging="block.key === draggingKey"
                :draggable="props.canManage && !props.pasting"
                :title="blockTitle(block)"
                @click.stop="onChipClick(block)"
                @dragstart="onChipDragStart(block, $event)"
                @dragend="onChipDragEnd"
              />
              <span
                v-if="cellExtraCount(room.id, column.dayIndex, column.periodIndex) > 0"
                class="shrink-0 pl-1 text-[9.5px] leading-3 text-faint"
              >
                +{{ cellExtraCount(room.id, column.dayIndex, column.periodIndex) }} 节
              </span>

              <!-- 框选高亮（pointer-events-none，不影响块拖拽/点击） -->
              <div
                v-if="isCellSelected(room.id, column)"
                class="pointer-events-none absolute inset-0 bg-accent/10 ring-2 ring-inset ring-accent/60"
              />

              <!-- 粘贴目标矩形虚线预览 -->
              <div
                v-if="isPastePreviewCell(room.id, column)"
                class="pointer-events-none absolute inset-0 z-[3] border-2 border-dashed border-accent bg-accent/5"
              />
            </div>
          </div>
        </div>
      </div>

      <footer
        class="flex h-8 shrink-0 items-center gap-3 border-t border-line bg-surface px-3.5 text-[10px] text-faint"
      >
        <span class="shrink-0">
          {{ sortedRooms.length }} 间教室 · {{ weekBlocks.length }} 节课
        </span>
        <span class="ml-auto truncate">
          {{
            props.canManage
              ? '点击空格新建 · 拖拽空白处框选多格（可复制/粘贴/批量应用/清除） · 点击房间行头选整行 · 拖拽课块跨房间 / 跨天移动'
              : '只读模式 · 横向滚动查看更多日期'
          }}
        </span>
      </footer>
    </template>
  </div>
</template>

<style scoped>
/* 同一格堆叠多块时压缩为「标题 + 冲突徽标」两行，两块都能完整显示。 */
.mx-cell.is-dense :deep(.board-chip) {
  flex: 1 1 0;
  min-height: 0;
  gap: 0;
  padding: 0 4px;
}

.mx-cell.is-dense :deep(.chip-main) {
  gap: 2px;
}

.mx-cell.is-dense :deep(.chip-course) {
  font-size: 10px;
  line-height: 12px;
}

.mx-cell.is-dense :deep(.chip-badges),
.mx-cell.is-dense :deep(.chip-meta) {
  display: none;
}

.mx-cell.is-dense :deep(.chip-conflict) {
  margin-top: 0;
  padding: 0 2px;
  border-radius: 2px;
  font-size: 8px;
  line-height: 10px;
}
</style>
