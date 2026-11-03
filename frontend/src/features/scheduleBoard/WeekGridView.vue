<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

import AppIcon from '@/components/ui/AppIcon.vue'

import {
  BOARD_PERIODS,
  courseColor,
  formatMonthDay,
  isoWeekday,
  periodPlacement,
  weekDates,
  weekdayName,
  type BoardBlock,
  type BoardRoom,
} from './boardTypes'
import { durationMinutes, periodDropRange } from './boardActions'
import ScheduleBlockChip from './ScheduleBlockChip.vue'

const props = withDefaults(
  defineProps<{
    weekStart: string
    rooms: BoardRoom[]
    blocks: BoardBlock[]
    selectedKey: string | null
    canManage: boolean
  }>(),
  {},
)

const emit = defineEmits<{
  select: [block: BoardBlock]
  create: [request: { room_id: string | null; date: string; start_time: string; end_time: string }]
  move: [request: { block: BoardBlock; day_of_week: number; start_time: string; end_time: string }]
}>()

const MAX_VISIBLE = 3

const dates = computed(() => weekDates(props.weekStart))
const draggingKey = ref<string | null>(null)
const dragOverKey = ref<string | null>(null)
const expandedCell = ref<{ date: string; periodIndex: number } | null>(null)

let suppressClickUntil = 0

const todayValue = (() => {
  const now = new Date()
  const month = String(now.getMonth() + 1).padStart(2, '0')
  const day = String(now.getDate()).padStart(2, '0')
  return `${now.getFullYear()}-${month}-${day}`
})()

const cellMap = computed(() => {
  const map = new Map<string, BoardBlock[]>()
  for (const block of props.blocks) {
    const index = periodPlacement(block.start_time).index
    const key = `${block.date}|${index}`
    const list = map.get(key) ?? []
    list.push(block)
    map.set(key, list)
  }
  for (const list of map.values()) {
    list.sort(
      (left, right) =>
        Number(right.conflicts.length > 0) - Number(left.conflicts.length > 0) ||
        left.start_time.localeCompare(right.start_time) ||
        left.room_code.localeCompare(right.room_code),
    )
  }
  return map
})

/** 每格最多展示 MAX_VISIBLE 个课块，其余折叠为 +N（+N = 总课块数 − 展示数）。 */
const cellView = computed(() => {
  const map = new Map<string, { blocks: BoardBlock[]; rest: number }>()
  for (const [key, list] of cellMap.value) {
    const visible = list.slice(0, MAX_VISIBLE)
    map.set(key, { blocks: visible, rest: list.length - visible.length })
  }
  return map
})

const expandedBlocks = computed(() => {
  const cell = expandedCell.value
  if (!cell) return []
  return cellMap.value.get(`${cell.date}|${cell.periodIndex}`) ?? []
})

const legends = computed(() => {
  const seen = new Map<string, { fg: string; bg: string }>()
  for (const block of props.blocks) {
    if (!seen.has(block.course_name)) seen.set(block.course_name, courseColor(block.course_name))
    if (seen.size >= 8) break
  }
  return [...seen.entries()].map(([course, color]) => ({ course, color }))
})

function cellBlocks(date: string, periodIndex: number): BoardBlock[] {
  return cellMap.value.get(`${date}|${periodIndex}`) ?? []
}

function viewOf(date: string, periodIndex: number): { blocks: BoardBlock[]; rest: number } {
  return cellView.value.get(`${date}|${periodIndex}`) ?? { blocks: [], rest: 0 }
}

function isToday(date: string): boolean {
  return date === todayValue
}

function openCell(date: string, periodIndex: number) {
  expandedCell.value = { date, periodIndex }
}

function onCellClick(date: string, periodIndex: number) {
  if (Date.now() < suppressClickUntil) return
  if (expandedCell.value) return
  const period = BOARD_PERIODS[periodIndex]
  if (!period) return
  emit('create', {
    room_id: null,
    date,
    start_time: period.start,
    end_time: period.end,
  })
}

function onDragStart(event: DragEvent, block: BoardBlock) {
  if (!props.canManage || block.source !== 'template') {
    event.preventDefault()
    return
  }
  draggingKey.value = block.key
  suppressClickUntil = Date.now() + 300
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', block.key)
  }
}

function onDragEnd() {
  draggingKey.value = null
  dragOverKey.value = null
  suppressClickUntil = Date.now() + 200
}

function onCellDragOver(event: DragEvent, key: string) {
  if (!props.canManage || !draggingKey.value) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
  dragOverKey.value = key
}

function clearDragOver() {
  dragOverKey.value = null
}

function onCellDrop(event: DragEvent, date: string, periodIndex: number) {
  event.preventDefault()
  const key = event.dataTransfer?.getData('text/plain') || draggingKey.value
  draggingKey.value = null
  dragOverKey.value = null
  if (!props.canManage || !key) return
  const block = props.blocks.find((item) => item.key === key)
  if (!block || block.source !== 'template') return
  const span = durationMinutes(block.start_time, block.end_time)
  const range = periodDropRange(periodIndex, span > 0 ? span : 100)
  const dayOfWeek = isoWeekday(date)
  suppressClickUntil = Date.now() + 300
  if (
    block.day_of_week === dayOfWeek &&
    block.start_time === range.start_time &&
    block.end_time === range.end_time
  ) {
    return
  }
  emit('move', {
    block,
    day_of_week: dayOfWeek,
    start_time: range.start_time,
    end_time: range.end_time,
  })
}

function selectBlock(block: BoardBlock) {
  if (Date.now() < suppressClickUntil) return
  expandedCell.value = null
  emit('select', block)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && expandedCell.value) expandedCell.value = null
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-surface">
    <div class="min-h-0 flex-1 overflow-y-auto">
      <div
        class="grid w-full"
        :style="{
          gridTemplateColumns: '88px repeat(7, minmax(0, 1fr))',
          gridTemplateRows: '36px repeat(5, minmax(112px, 1fr))',
          minHeight: 'max(100%, 596px)',
        }"
      >
        <div
          class="sticky top-0 z-10 flex items-center border-b border-r border-line bg-[#fbfcfd] pl-3 text-[10.5px] text-faint"
        >
          节次 / 时间
        </div>
        <div
          v-for="date in dates"
          :key="`head-${date}`"
          class="sticky top-0 z-10 flex items-center justify-center gap-1.5 border-b border-r border-line bg-[#fbfcfd] text-[11.5px] font-medium text-ink-soft last:border-r-0"
          :class="isoWeekday(date) >= 6 ? 'bg-[#f9fafb] text-muted' : ''"
        >
          <span>{{ weekdayName(isoWeekday(date)) }}</span>
          <span class="font-mono text-[10px] font-normal text-faint">{{
            formatMonthDay(date)
          }}</span>
          <span
            v-if="isToday(date)"
            class="rounded-sm bg-accent-soft px-1 text-[9px] font-medium text-accent"
            >今天</span
          >
        </div>

        <template v-for="(period, periodIndex) in BOARD_PERIODS" :key="period.key">
          <div
            class="flex flex-col justify-center gap-1 border-b border-r border-line bg-[#fbfcfd] px-2.5"
          >
            <span class="whitespace-nowrap text-[11px] font-medium text-ink-soft">{{
              period.label
            }}</span>
            <span class="whitespace-nowrap font-mono text-[9.5px] text-faint"
              >{{ period.start }}–{{ period.end }}</span
            >
          </div>
          <div
            v-for="date in dates"
            :key="`cell-${date}-${period.key}`"
            class="relative flex flex-col gap-1 overflow-hidden border-b border-r border-line p-1 transition-colors last:border-r-0"
            :data-date="date"
            :data-period="periodIndex"
            :class="[
              isoWeekday(date) >= 6 ? 'bg-[#fcfcfd]' : 'bg-surface',
              dragOverKey === `${date}|${periodIndex}`
                ? 'bg-accent-soft ring-2 ring-inset ring-accent'
                : '',
            ]"
            @click="onCellClick(date, periodIndex)"
            @dragover="onCellDragOver($event, `${date}|${periodIndex}`)"
            @dragleave="clearDragOver"
            @drop="onCellDrop($event, date, periodIndex)"
          >
            <div
              v-for="(block, blockIndex) in viewOf(date, periodIndex).blocks"
              :key="block.key"
              class="week-chip shrink-0"
              :class="{
                'cursor-pointer': true,
                'pr-8':
                  blockIndex === viewOf(date, periodIndex).blocks.length - 1 &&
                  viewOf(date, periodIndex).rest > 0,
                'opacity-45': draggingKey === block.key,
                'cursor-grab': canManage && block.source === 'template',
              }"
              :draggable="canManage && block.source === 'template'"
              @click.stop="selectBlock(block)"
              @dragstart="onDragStart($event, block)"
              @dragend="onDragEnd"
            >
              <ScheduleBlockChip
                :block="block"
                compact
                :selected="block.key === selectedKey"
                :dragging="draggingKey === block.key"
              />
            </div>

            <button
              v-if="viewOf(date, periodIndex).rest > 0"
              type="button"
              class="absolute bottom-1 right-1 z-10 rounded border border-line bg-surface/95 px-1 text-[9px] font-medium leading-[15px] text-muted shadow-sm transition-colors hover:text-accent"
              :title="`查看该时段的 ${cellBlocks(date, periodIndex).length} 个课块`"
              @click.stop="openCell(date, periodIndex)"
            >
              +{{ viewOf(date, periodIndex).rest }}
            </button>
          </div>
        </template>
      </div>
    </div>

    <div
      class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-t border-line px-3 py-1.5"
    >
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span v-for="legend in legends" :key="legend.course" class="flex items-center gap-1">
          <span class="h-1.5 w-1.5 rounded-sm" :style="{ background: legend.color.fg }" />
          <span class="text-[9.5px] text-muted">{{ legend.course }}</span>
        </span>
        <span v-if="!legends.length" class="text-[9.5px] text-faint">本周暂无课程</span>
      </div>
      <div class="ml-auto flex items-center gap-3 text-[9.5px] text-faint">
        <span class="flex items-center gap-1">
          <span class="h-2 w-2 rounded-sm bg-rose-500" />房间冲突
        </span>
        <span class="flex items-center gap-1">
          <span class="h-2 w-2 rounded-sm bg-amber-500" />教师冲突
        </span>
        <span class="flex items-center gap-1">
          <AppIcon name="info" :size="11" />
          每格最多展示 {{ MAX_VISIBLE }} 个课块，超出显示 +N
        </span>
        <span v-if="canManage" class="flex items-center gap-1">
          <AppIcon name="layers" :size="11" />
          可拖拽课块调整星期 / 节次
        </span>
      </div>
    </div>

    <!-- +N 展开浮层 -->
    <div
      v-if="expandedCell"
      class="fixed inset-0 z-40 flex items-center justify-center bg-ink/25 p-6 backdrop-blur-[1px]"
      @mousedown.self="expandedCell = null"
    >
      <div
        class="flex max-h-[70vh] w-[380px] flex-col overflow-hidden rounded-xl border border-line bg-surface shadow-[0_24px_70px_-24px_rgba(16,24,40,0.4)]"
      >
        <div class="flex items-center justify-between border-b border-line px-4 py-3">
          <div>
            <p class="text-[12.5px] font-semibold text-ink">
              {{ formatMonthDay(expandedCell.date) }}
              {{ weekdayName(isoWeekday(expandedCell.date)) }} ·
              {{ BOARD_PERIODS[expandedCell.periodIndex]?.label }}
            </p>
            <p class="mt-0.5 text-[10.5px] text-faint">共 {{ expandedBlocks.length }} 个课块</p>
          </div>
          <button
            type="button"
            aria-label="关闭"
            class="flex h-7 w-7 items-center justify-center rounded-md text-faint hover:bg-canvas hover:text-ink"
            @click="expandedCell = null"
          >
            <AppIcon name="x" :size="14" />
          </button>
        </div>
        <div class="flex flex-col gap-1.5 overflow-y-auto p-3">
          <button
            v-for="block in expandedBlocks"
            :key="block.key"
            type="button"
            class="text-left"
            @click="selectBlock(block)"
          >
            <ScheduleBlockChip :block="block" />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 周网格里去掉冲突文案行（保留红/琥珀描边与斜纹），让 5 个节次在 900 高度内全部可见。 */
.week-chip :deep(.chip-conflict) {
  display: none;
}
</style>
