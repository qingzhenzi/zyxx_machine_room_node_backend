<script setup lang="ts">
/**
 * B · 资源时间轴（单日）。
 *
 * 行 = 房间（按区域分组，组头/房间列粘性），X 轴 = 08:00–21:00（整点刻度，按分钟比例定位）。
 * 纯展示组件：数据全部来自 props，编辑意图（点击空白新建 / 拖动课块改时间与房间 / 选中）
 * 只用 emits 抛给页面容器，不 fetch、不写全局状态。
 *
 * 拖拽用原生 HTML5 DnD：横向吸附 10 分钟、纵向换房间；落点与同房间其它课块重叠时
 * 显示红色预览且松手不 emit。同一行重叠课块用错位堆叠（较早的占全高，后续下移只露标题）。
 */
import { computed, ref } from 'vue'
import type { CSSProperties } from 'vue'

import EmptyState from '@/components/ui/EmptyState.vue'

import { groupRoomsByArea, type BoardBlock, type BoardRoom } from './boardTypes'
import ScheduleBlockChip from './ScheduleBlockChip.vue'

interface TimelineCreatePayload {
  room_id: string
  date: string
  start_time: string
  end_time: string
}

interface TimelineMovePayload {
  block: BoardBlock
  room_id: string
  start_time: string
  end_time: string
}

interface PreviewState {
  room_id: string
  start: number
  end: number
  valid: boolean
}

interface PositionedBlock {
  block: BoardBlock
  compact: boolean
  lane: number
  style: CSSProperties
}

interface HourTick {
  label: string
  left: number
  transform: string
}

const DAY_START_MIN = 8 * 60
const DAY_END_MIN = 21 * 60
const DAY_SPAN_MIN = DAY_END_MIN - DAY_START_MIN
const SNAP_MIN = 10
const DEFAULT_DURATION_MIN = 100
const BLOCK_INSET_PX = 5
const LANE_STEP_PX = 22
const COMPACT_MAX_DURATION_MIN = 70

const props = defineProps<{
  date: string
  rooms: BoardRoom[]
  blocks: BoardBlock[]
  selectedKey: string | null
  canManage: boolean
}>()

const emit = defineEmits<{
  select: [block: BoardBlock]
  create: [payload: TimelineCreatePayload]
  move: [payload: TimelineMovePayload]
}>()

const dragBlock = ref<BoardBlock | null>(null)
const preview = ref<PreviewState | null>(null)
const dropRoomId = ref<string | null>(null)

const draggingKey = computed(() => dragBlock.value?.key ?? '')

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

function parseMinutes(time: string): number {
  const [hour, minute] = time.split(':').map(Number)
  return (hour ?? 0) * 60 + (minute ?? 0)
}

function formatMinutes(minutes: number): string {
  const safe = clamp(Math.round(minutes), 0, 24 * 60 - 1)
  const hour = Math.floor(safe / 60)
  const minute = safe % 60
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

/** 当天课块（父级可能传全周，这里再做一次夹取，重复过滤安全）。 */
const dayBlocks = computed(() => props.blocks.filter((block) => block.date === props.date))

const groupedRooms = computed(() => groupRoomsByArea(props.rooms))

const roomBlockCounts = computed(() => {
  const counts = new Map<string, number>()
  for (const block of dayBlocks.value) {
    counts.set(block.room_id, (counts.get(block.room_id) ?? 0) + 1)
  }
  return counts
})

function groupBlockCount(rooms: BoardRoom[]): number {
  return rooms.reduce((total, room) => total + (roomBlockCounts.value.get(room.id) ?? 0), 0)
}

const hourTicks = computed<HourTick[]>(() =>
  Array.from({ length: 14 }, (_, index) => ({
    label: formatMinutes(DAY_START_MIN + index * 60),
    left: (index / 13) * 100,
    transform: index === 0 ? 'none' : index === 13 ? 'translateX(-100%)' : 'translateX(-50%)',
  })),
)

const trackGridStyle: CSSProperties = {
  backgroundImage:
    'linear-gradient(to right, var(--color-line) 0 1px, transparent 1px), linear-gradient(to right, #f1f3f8 0 1px, transparent 1px)',
  backgroundSize: 'calc(100% / 13) 100%, calc(100% / 26) 100%',
  backgroundRepeat: 'repeat-x',
}

/** 课块在 08:00–21:00 内的百分比几何（超出画布范围的时段做夹取，保证至少可见 10 分钟）。 */
function blockGeometry(block: BoardBlock): { left: number; width: number } {
  const start = parseMinutes(block.start_time)
  const end = parseMinutes(block.end_time)
  const startOffset = clamp(start - DAY_START_MIN, 0, DAY_SPAN_MIN - SNAP_MIN)
  const endOffset = clamp(
    Math.max(end, start + SNAP_MIN) - DAY_START_MIN,
    startOffset + SNAP_MIN,
    DAY_SPAN_MIN,
  )
  return {
    left: (startOffset / DAY_SPAN_MIN) * 100,
    width: ((endOffset - startOffset) / DAY_SPAN_MIN) * 100,
  }
}

function blockStyle(block: BoardBlock, lane: number): CSSProperties {
  const { left, width } = blockGeometry(block)
  const laneOffset = lane === 0 ? 0 : LANE_STEP_PX + Math.min(lane - 1, 1) * (LANE_STEP_PX - 4)
  return {
    left: `${left.toFixed(3)}%`,
    width: `${width.toFixed(3)}%`,
    top: `${BLOCK_INSET_PX + laneOffset}px`,
    bottom: `${BLOCK_INSET_PX}px`,
    zIndex: String(2 + lane),
  }
}

/**
 * 同一房间内重叠课块的错位堆叠：
 * 较早的课块占满行高，后续课块下移 22px（多重叠再下移 18px）只露标题。
 */
const positionedByRoom = computed(() => {
  const grouped = new Map<string, BoardBlock[]>()
  for (const block of dayBlocks.value) {
    const list = grouped.get(block.room_id)
    if (list) list.push(block)
    else grouped.set(block.room_id, [block])
  }

  const result = new Map<string, PositionedBlock[]>()
  for (const [roomId, list] of grouped) {
    const sorted = [...list].sort(
      (left, right) =>
        parseMinutes(left.start_time) - parseMinutes(right.start_time) ||
        parseMinutes(left.end_time) - parseMinutes(right.end_time),
    )
    let clusterEnd = -1
    let lane = 0
    const items: PositionedBlock[] = []
    for (const block of sorted) {
      const start = parseMinutes(block.start_time)
      const end = parseMinutes(block.end_time)
      if (start >= clusterEnd) {
        clusterEnd = end
        lane = 0
      } else {
        lane += 1
        clusterEnd = Math.max(clusterEnd, end)
      }
      items.push({
        block,
        compact: end - start < COMPACT_MAX_DURATION_MIN,
        lane,
        style: blockStyle(block, lane),
      })
    }
    result.set(roomId, items)
  }
  return result
})

function rowBlocks(roomId: string): PositionedBlock[] {
  return positionedByRoom.value.get(roomId) ?? []
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

/** 区域名可能已自带「区域」后缀（后端 `area_name` = "P3 区域"），避免出现「P3 区域 区域」。 */
function areaLabel(name: string): string {
  const trimmed = name.trim()
  return trimmed.endsWith('区域') ? trimmed : `${trimmed} 区域`
}

function deviceState(room: BoardRoom): { className: string; label: string } {
  if (!room.device) return { className: 'bg-line-strong', label: '未绑定设备' }
  if (room.device.online === true) return { className: 'bg-emerald-500', label: '设备在线' }
  if (room.device.online === false) return { className: 'bg-rose-400', label: '设备离线' }
  return { className: 'bg-faint', label: '设备在线状态未知' }
}

const previewGeometry = computed<CSSProperties | null>(() => {
  const state = preview.value
  if (!state) return null
  const startOffset = clamp(state.start - DAY_START_MIN, 0, DAY_SPAN_MIN)
  const endOffset = clamp(state.end - DAY_START_MIN, startOffset, DAY_SPAN_MIN)
  return {
    left: `${(startOffset / DAY_SPAN_MIN) * 100}%`,
    width: `${((endOffset - startOffset) / DAY_SPAN_MIN) * 100}%`,
  }
})

function hasRoomOverlap(roomId: string, excludeKey: string, start: number, end: number): boolean {
  for (const block of dayBlocks.value) {
    if (block.room_id !== roomId || block.key === excludeKey) continue
    if (parseMinutes(block.start_time) < end && start < parseMinutes(block.end_time)) return true
  }
  return false
}

/** 根据指针位置换算「吸附到 10 分钟 + 保留时长」的落点，并刷新拖拽预览。 */
function updatePreview(roomId: string, event: DragEvent, track: HTMLElement): PreviewState | null {
  const block = dragBlock.value
  if (!block) return null
  const rect = track.getBoundingClientRect()
  if (rect.width <= 0) return null
  const duration = Math.max(SNAP_MIN, parseMinutes(block.end_time) - parseMinutes(block.start_time))
  const ratio = clamp((event.clientX - rect.left) / rect.width, 0, 1)
  const rawStart = DAY_START_MIN + ratio * DAY_SPAN_MIN
  const start = clamp(
    Math.round(rawStart / SNAP_MIN) * SNAP_MIN,
    DAY_START_MIN,
    DAY_END_MIN - Math.min(duration, DAY_SPAN_MIN - SNAP_MIN),
  )
  const end = Math.min(start + duration, DAY_END_MIN)
  const next: PreviewState = {
    room_id: roomId,
    start,
    end,
    valid: !hasRoomOverlap(roomId, block.key, start, end),
  }
  preview.value = next
  dropRoomId.value = roomId
  return next
}

function clearDrag(): void {
  dragBlock.value = null
  preview.value = null
  dropRoomId.value = null
}

function onBlockDragStart(block: BoardBlock, event: DragEvent): void {
  if (!props.canManage) {
    event.preventDefault()
    return
  }
  dragBlock.value = block
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', block.key)
  }
}

function onBlockDragEnd(): void {
  clearDrag()
}

function onTrackDragOver(roomId: string, event: DragEvent): void {
  if (!props.canManage || !dragBlock.value) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
  const track = event.currentTarget as HTMLElement | null
  if (track) updatePreview(roomId, event, track)
}

function onTrackDragLeave(roomId: string, event: DragEvent): void {
  const track = event.currentTarget as HTMLElement | null
  const related = event.relatedTarget as Node | null
  if (track && related && track.contains(related)) return
  if (preview.value?.room_id === roomId) preview.value = null
}

function onTrackDrop(roomId: string, event: DragEvent): void {
  if (!props.canManage) return
  const block = dragBlock.value
  const track = event.currentTarget as HTMLElement | null
  const state = track ? updatePreview(roomId, event, track) : preview.value
  clearDrag()
  if (!block || !state || !state.valid) return
  const sameRoom = block.room_id === roomId
  const sameStart = parseMinutes(block.start_time) === state.start
  const sameEnd = parseMinutes(block.end_time) === state.end
  if (sameRoom && sameStart && sameEnd) return
  emit('move', {
    block,
    room_id: roomId,
    start_time: formatMinutes(state.start),
    end_time: formatMinutes(state.end),
  })
}

/** 点击行内时间轴空白：从落点向上取整到 10 分钟，默认 100 分钟。 */
function onTrackClick(roomId: string, event: MouseEvent): void {
  if (!props.canManage || dragBlock.value) return
  const track = event.currentTarget as HTMLElement | null
  if (!track) return
  const rect = track.getBoundingClientRect()
  if (rect.width <= 0) return
  const ratio = clamp((event.clientX - rect.left) / rect.width, 0, 1)
  const rawStart = DAY_START_MIN + ratio * DAY_SPAN_MIN
  const start = clamp(
    Math.ceil(rawStart / SNAP_MIN) * SNAP_MIN,
    DAY_START_MIN,
    DAY_END_MIN - DEFAULT_DURATION_MIN,
  )
  emit('create', {
    room_id: roomId,
    date: props.date,
    start_time: formatMinutes(start),
    end_time: formatMinutes(start + DEFAULT_DURATION_MIN),
  })
}
</script>

<template>
  <div class="flex h-full min-h-[420px] min-w-0 flex-col overflow-hidden bg-surface">
    <EmptyState
      v-if="groupedRooms.length === 0"
      class="my-auto"
      title="暂无可排课房间"
      description="当前筛选条件下没有房间。清除区域或房间筛选后，这里会按区域列出当天各房间的时间轴。"
      icon="calendar"
    />

    <div v-else class="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
      <div class="flex min-h-full flex-col">
        <!-- 时间刻度（粘性） -->
        <div class="sticky top-0 z-30 flex h-9 items-stretch border-b border-line bg-[#fbfcfd]">
          <div
            class="flex w-[172px] shrink-0 items-center border-r border-line px-3.5 text-[10.5px] text-faint"
          >
            <span>房间 · 时间</span>
          </div>
          <div class="relative min-w-0 flex-1">
            <span
              v-for="tick in hourTicks"
              :key="tick.label"
              class="absolute top-0 whitespace-nowrap pt-2.5 font-mono text-[9.5px] text-faint"
              :style="{ left: `${tick.left}%`, transform: tick.transform }"
            >
              {{ tick.label }}
            </span>
          </div>
        </div>

        <!-- 无课提示（有房间无课，避免白板） -->
        <div
          v-if="dayBlocks.length === 0 && props.blocks.length > 0"
          class="flex h-7 items-center gap-2 border-b border-line bg-accent-soft/40 px-3.5 text-[11px] text-muted"
        >
          <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"></span>
          <span class="truncate">
            今日暂无课程{{
              props.canManage ? '：点击房间行的时间轴空白处即可新建（默认 100 分钟）。' : '。'
            }}
          </span>
        </div>

        <div>
          <div
            v-for="(group, groupIndex) in groupedRooms"
            :key="`${group.rooms[0]?.area_id ?? group.area}-${groupIndex}`"
          >
            <!-- 区域组头（粘性） -->
            <div
              class="sticky top-9 z-20 flex h-[26px] items-center gap-2 border-y border-line bg-[#f7f8fa] px-3.5 text-[10.5px] font-semibold text-muted"
            >
              <span class="h-[10px] w-[3px] shrink-0 rounded-sm bg-line-strong"></span>
              <span class="shrink-0">{{ areaLabel(group.area) }}</span>
              <span class="truncate font-normal text-faint">
                · {{ group.rooms.length }} 间教室 · 今日 {{ groupBlockCount(group.rooms) }} 节
              </span>
            </div>

            <div>
              <div
                v-for="room in group.rooms"
                :key="room.id"
                class="flex h-16 border-b border-line/60 bg-surface"
              >
                <!-- 房间列（粘性） -->
                <div
                  class="sticky left-0 z-10 flex w-[172px] shrink-0 flex-col justify-center gap-0.5 border-r border-line px-3.5"
                  :class="dropRoomId === room.id && dragBlock ? 'bg-accent-soft' : 'bg-surface'"
                >
                  <div class="flex min-w-0 items-baseline gap-1.5">
                    <span
                      class="min-w-0 truncate font-mono text-[11.5px] leading-4 font-semibold text-ink"
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

                <!-- 时间轨道 -->
                <div
                  class="relative min-w-0 flex-1 overflow-hidden"
                  :data-room-code="room.code"
                  :class="[
                    preview && preview.room_id === room.id ? 'bg-accent-soft/25' : '',
                    props.canManage ? 'cursor-crosshair' : 'cursor-default',
                  ]"
                  :style="trackGridStyle"
                  @click="onTrackClick(room.id, $event)"
                  @dragover="onTrackDragOver(room.id, $event)"
                  @dragleave="onTrackDragLeave(room.id, $event)"
                  @drop="onTrackDrop(room.id, $event)"
                >
                  <div
                    v-for="item in rowBlocks(room.id)"
                    :key="item.block.key"
                    class="absolute overflow-hidden rounded-[5px]"
                    :data-block-key="item.block.key"
                    :class="
                      props.canManage ? 'cursor-grab active:cursor-grabbing' : 'cursor-pointer'
                    "
                    :style="item.style"
                    :title="blockTitle(item.block)"
                    :draggable="props.canManage"
                    @click.stop="emit('select', item.block)"
                    @dragstart="onBlockDragStart(item.block, $event)"
                    @dragend="onBlockDragEnd"
                  >
                    <ScheduleBlockChip
                      class="h-full w-full"
                      :block="item.block"
                      :compact="item.compact"
                      :show-room="false"
                      :selected="item.block.key === props.selectedKey"
                      :dragging="item.block.key === draggingKey"
                    />
                  </div>

                  <!-- 拖拽落点预览 -->
                  <div
                    v-if="preview && preview.room_id === room.id"
                    class="pointer-events-none absolute inset-y-[5px] z-20 flex items-center justify-center overflow-hidden rounded-[5px] border"
                    :class="
                      preview.valid
                        ? 'border-accent bg-accent/15 text-accent'
                        : 'border-rose-400 bg-rose-100/70 text-rose-600'
                    "
                    :style="previewGeometry ?? undefined"
                  >
                    <span class="truncate px-1 text-[10px] font-medium">
                      {{ formatMinutes(preview.start) }}–{{ formatMinutes(preview.end)
                      }}<template v-if="!preview.valid"> · 与同房间课程重叠</template>
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
