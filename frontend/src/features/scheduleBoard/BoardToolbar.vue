<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink } from 'vue-router'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'

import {
  formatMonthDay,
  formatWeekRange,
  weekdayName,
  type BoardArea,
  type BoardConflict,
  type BoardRoom,
  type BoardViewMode,
  weekStartOf,
} from './boardTypes'

const props = withDefaults(
  defineProps<{
    weekStart: string
    selectedDate: string
    areas: BoardArea[]
    rooms: BoardRoom[]
    conflicts: BoardConflict[]
    loading?: boolean
    canManage?: boolean
    canUndo?: boolean
    canRedo?: boolean
    undoLabel?: string
    redoLabel?: string
  }>(),
  {
    loading: false,
    canManage: false,
    canUndo: false,
    canRedo: false,
    undoLabel: '',
    redoLabel: '',
  },
)

const viewMode = defineModel<BoardViewMode>('viewMode', { required: true })
const areaId = defineModel<string>('areaId', { required: true })
const roomId = defineModel<string>('roomId', { required: true })
const keyword = defineModel<string>('keyword', { required: true })
const onlyConflicts = defineModel<boolean>('onlyConflicts', { required: true })
/** B 时间轴的调整作用域：本次 = 只写当天覆盖；模板 = 改周模板。 */
const scope = defineModel<'template' | 'occurrence'>('scope', { required: true })

const emit = defineEmits<{
  prevWeek: []
  nextWeek: []
  prevDay: []
  nextDay: []
  today: []
  refresh: []
  create: []
  undo: []
  redo: []
  locate: [conflict: BoardConflict]
}>()

const VIEW_OPTIONS: Array<{ value: BoardViewMode; label: string; icon: string }> = [
  { value: 'week', label: '周网格', icon: 'calendar' },
  { value: 'timeline', label: '时间轴', icon: 'clock' },
  { value: 'matrix', label: '编排矩阵', icon: 'overview' },
]

const SCOPE_OPTIONS: Array<{
  value: 'occurrence' | 'template'
  label: string
  title: string
}> = [
  {
    value: 'occurrence',
    label: '本次',
    title: '拖拽/新建只调整当天这一节课，写单日覆盖；其它周仍按模板',
  },
  {
    value: 'template',
    label: '模板',
    title: '拖拽/新建修改周模板，影响整个生效区间；「本次停课/恢复本次」仍只影响当天',
  },
]

const conflictOpen = ref(false)

const undoTitle = computed(() =>
  props.undoLabel ? `撤销：${props.undoLabel}（Ctrl+Z）` : '撤销（Ctrl+Z）',
)
const redoTitle = computed(() =>
  props.redoLabel ? `重做：${props.redoLabel}（Ctrl+Shift+Z）` : '重做（Ctrl+Shift+Z）',
)

const roomGroups = computed(() => {
  const groups = new Map<string, BoardRoom[]>()
  for (const room of [...props.rooms].sort(
    (left, right) =>
      left.area_name.localeCompare(right.area_name, 'zh-CN') ||
      left.code.localeCompare(right.code, 'zh-CN'),
  )) {
    const list = groups.get(room.area_name) ?? []
    list.push(room)
    groups.set(room.area_name, list)
  }
  return [...groups.entries()].map(([area, rooms]) => ({ area, rooms }))
})

const weekLabel = computed(() => {
  const year = Number(props.weekStart.slice(0, 4))
  return `${year} 第 ${isoWeekNumber(props.weekStart)} 周 · ${formatWeekRange(props.weekStart)}`
})

const dayLabel = computed(
  () => `${formatMonthDay(props.selectedDate)} ${weekdayName(isoWeekday(props.selectedDate))}`,
)

const isCurrentWeek = computed(
  () => props.weekStart === weekStartOf(new Date().toISOString().slice(0, 10)),
)

function isoWeekday(date: string): number {
  const value = new Date(`${date}T00:00:00Z`)
  return ((value.getUTCDay() + 6) % 7) + 1
}

/** ISO 8601 周序号（设计基线周导航里的「第 N 周」）。 */
function isoWeekNumber(date: string): number {
  const target = new Date(`${date}T00:00:00Z`)
  const weekday = (target.getUTCDay() + 6) % 7
  target.setUTCDate(target.getUTCDate() - weekday + 3)
  const firstThursday = new Date(Date.UTC(target.getUTCFullYear(), 0, 4))
  const firstWeekday = (firstThursday.getUTCDay() + 6) % 7
  firstThursday.setUTCDate(firstThursday.getUTCDate() - firstWeekday + 3)
  return 1 + Math.round((target.getTime() - firstThursday.getTime()) / (7 * 86_400_000))
}

function selectConflict(conflict: BoardConflict) {
  conflictOpen.value = false
  emit('locate', conflict)
}

function jumpToday() {
  emit('today')
}
</script>

<template>
  <div
    class="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-b border-line bg-surface px-2.5 py-2"
  >
    <!-- 视图切换 -->
    <div class="inline-flex shrink-0 items-center gap-0.5 rounded-lg bg-canvas p-[3px]">
      <button
        v-for="option in VIEW_OPTIONS"
        :key="option.value"
        type="button"
        role="tab"
        :aria-selected="viewMode === option.value"
        class="flex h-7 items-center gap-1.5 rounded-md px-2 text-[11.5px] transition-colors"
        :class="
          viewMode === option.value
            ? 'bg-surface font-semibold text-accent shadow-sm'
            : 'font-medium text-muted hover:text-ink-soft'
        "
        @click="viewMode = option.value"
      >
        <AppIcon :name="option.icon" :size="13" />
        <span>{{ option.label }}</span>
      </button>
    </div>

    <span class="h-5 w-px shrink-0 bg-line" />

    <!-- 周导航（周网格 / 编排矩阵） -->
    <div v-if="viewMode !== 'timeline'" class="flex shrink-0 items-center gap-0.5">
      <IconButton
        icon="chevron-left"
        label="上一周"
        size="sm"
        :disabled="loading"
        @click="emit('prevWeek')"
      />
      <span class="px-1.5 text-[11.5px] font-medium tabular-nums text-ink-soft whitespace-nowrap">{{
        weekLabel
      }}</span>
      <IconButton
        icon="chevron-right"
        label="下一周"
        size="sm"
        :disabled="loading"
        @click="emit('nextWeek')"
      />
      <button
        type="button"
        class="h-7 shrink-0 rounded-md px-2 text-[11px] text-muted transition-colors hover:bg-canvas hover:text-ink disabled:opacity-50"
        :disabled="loading || isCurrentWeek"
        @click="jumpToday"
      >
        今天
      </button>
    </div>

    <!-- 单日导航（资源时间轴） -->
    <div v-else class="flex shrink-0 items-center gap-0.5">
      <IconButton
        icon="chevron-left"
        label="前一天"
        size="sm"
        :disabled="loading"
        @click="emit('prevDay')"
      />
      <span
        class="min-w-[86px] px-1.5 text-center text-[11.5px] font-medium tabular-nums text-ink-soft whitespace-nowrap"
        >{{ dayLabel }}</span
      >
      <IconButton
        icon="chevron-right"
        label="后一天"
        size="sm"
        :disabled="loading"
        @click="emit('nextDay')"
      />
      <button
        type="button"
        class="h-7 shrink-0 rounded-md px-2 text-[11px] text-muted transition-colors hover:bg-canvas hover:text-ink disabled:opacity-50"
        :disabled="loading"
        @click="jumpToday"
      >
        今天
      </button>
    </div>

    <!-- 作用域（仅资源时间轴：本次 = 单日覆盖，模板 = 改周模板） -->
    <div
      v-if="viewMode === 'timeline'"
      class="flex shrink-0 items-center gap-1.5"
      title="拖拽/新建的落库作用域：本次只写当天覆盖，模板修改周模板"
    >
      <span class="text-[10.5px] text-faint">作用域</span>
      <div
        class="inline-flex items-center gap-0.5 rounded-lg bg-canvas p-[2px]"
        role="group"
        aria-label="拖拽/新建作用域"
      >
        <button
          v-for="option in SCOPE_OPTIONS"
          :key="option.value"
          type="button"
          :aria-pressed="scope === option.value"
          :title="option.title"
          class="h-6 rounded-md px-2 text-[11px] transition-colors"
          :class="
            scope === option.value
              ? 'bg-surface font-semibold text-accent shadow-sm'
              : 'font-medium text-muted hover:text-ink-soft'
          "
          @click="scope = option.value"
        >
          {{ option.label }}
        </button>
      </div>
    </div>

    <span class="h-5 w-px shrink-0 bg-line" />

    <!-- 区域 / 房间筛选 -->
    <label class="flex shrink-0 items-center gap-1.5">
      <span class="text-[10.5px] text-faint">区域</span>
      <select v-model="areaId" class="select h-7 w-[116px] px-2 pr-6 text-[11.5px]">
        <option value="">全部区域</option>
        <option v-for="area in areas" :key="area.id" :value="area.id">{{ area.name }}</option>
      </select>
    </label>

    <label class="flex shrink-0 items-center gap-1.5">
      <span class="text-[10.5px] text-faint">房间</span>
      <select
        v-model="roomId"
        class="select h-7 w-[138px] px-2 pr-6 text-[11.5px]"
        :disabled="!rooms.length"
      >
        <option value="">全部房间</option>
        <optgroup v-for="group in roomGroups" :key="group.area" :label="group.area">
          <option v-for="room in group.rooms" :key="room.id" :value="room.id">
            {{ room.code }}
          </option>
        </optgroup>
      </select>
    </label>

    <div class="min-w-2 flex-1" />

    <SearchInput
      v-model="keyword"
      class="w-[172px] shrink-0"
      placeholder="搜索课程 / 教师 / 房间"
    />

    <!-- 操作区：窄屏时整组折到第二行并右对齐 -->
    <div class="ml-auto flex flex-wrap items-center gap-2">
      <!-- 只看冲突 + 冲突计数 -->
      <label class="flex shrink-0 items-center gap-1.5">
        <ToggleSwitch v-model="onlyConflicts" />
        <span class="text-[11px] text-muted whitespace-nowrap">只看冲突</span>
      </label>

      <div class="relative shrink-0">
        <button
          type="button"
          class="flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[11px] font-medium transition-colors"
          :class="
            conflicts.length
              ? 'border-rose-200 bg-rose-50 text-rose-700 hover:bg-rose-100'
              : 'border-line bg-surface text-muted hover:bg-canvas'
          "
          :aria-expanded="conflictOpen"
          title="查看本周冲突列表"
          @click="conflictOpen = !conflictOpen"
        >
          <span
            class="h-1.5 w-1.5 rounded-full"
            :class="conflicts.length ? 'bg-rose-500' : 'bg-slate-400'"
          />
          <span>冲突</span>
          <b class="tabular-nums">{{ conflicts.length }}</b>
        </button>

        <div v-if="conflictOpen" class="fixed inset-0 z-30" @click="conflictOpen = false" />
        <div
          v-if="conflictOpen"
          class="absolute right-0 top-[calc(100%+6px)] z-40 w-[320px] overflow-hidden rounded-lg border border-line bg-surface shadow-lg"
        >
          <div class="border-b border-line px-3 py-2 text-[11px] font-semibold text-ink">
            本周冲突 · {{ conflicts.length }} 处
          </div>
          <p v-if="!conflicts.length" class="px-3 py-4 text-[11px] text-muted">
            本周没有房间或教师时段冲突。
          </p>
          <ul v-else class="max-h-[260px] overflow-y-auto py-1">
            <li v-for="(conflict, index) in conflicts" :key="`${conflict.date}-${index}`">
              <button
                type="button"
                class="flex w-full items-start gap-2 px-3 py-2 text-left transition-colors hover:bg-canvas"
                @click="selectConflict(conflict)"
              >
                <span
                  class="mt-0.5 inline-flex shrink-0 rounded px-1 py-0.5 text-[9.5px] font-semibold"
                  :class="
                    conflict.severity === 'error'
                      ? 'bg-rose-50 text-rose-700'
                      : 'bg-amber-50 text-amber-700'
                  "
                >
                  {{ conflict.kind === 'room' ? '房间' : '教师' }}
                </span>
                <span class="min-w-0 text-[10.5px] leading-4 text-ink-soft">{{
                  conflict.message
                }}</span>
              </button>
            </li>
          </ul>
        </div>
      </div>

      <IconButton
        icon="rotate-ccw"
        :label="undoTitle"
        variant="outline"
        :disabled="!canUndo || loading"
        @click="emit('undo')"
      />

      <IconButton
        icon="rotate-cw"
        :label="redoTitle"
        variant="outline"
        :disabled="!canRedo || loading"
        @click="emit('redo')"
      />

      <IconButton
        icon="refresh"
        label="刷新排课数据"
        variant="outline"
        :loading="loading"
        :disabled="loading"
        @click="emit('refresh')"
      />

      <RouterLink
        :to="{ name: 'schedule-list' }"
        class="flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-[11.5px] text-muted transition-colors hover:bg-canvas hover:text-ink"
        title="查看旧版课表列表"
      >
        <AppIcon name="layers" :size="13" />
        <span>课表列表</span>
      </RouterLink>

      <AppButton v-if="canManage" size="sm" icon="plus" variant="primary" @click="emit('create')">
        新建课程
      </AppButton>
    </div>
  </div>
</template>
