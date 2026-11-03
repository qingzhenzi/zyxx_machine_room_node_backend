<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { shortId } from '@/utils/format'

import {
  BOARD_PERIODS,
  courseColor,
  formatMonthDay,
  periodIndexOf,
  weekdayName,
  type BoardBlock,
  type BoardRoom,
} from './boardTypes'
import { durationMinutes } from './boardActions'

const props = withDefaults(
  defineProps<{
    open: boolean
    block: BoardBlock | null
    rooms: BoardRoom[]
    canManage?: boolean
    deleting?: boolean
    /** 当前拖拽/新建作用域：B 默认「本次」，A/C 固定「模板」。 */
    scope?: 'template' | 'occurrence'
    /** 是否显示单日动作（仅 B 时间轴且有 manage 权限）。 */
    canAdjustOccurrence?: boolean
  }>(),
  { canManage: false, deleting: false, scope: 'template', canAdjustOccurrence: false },
)

const emit = defineEmits<{
  close: []
  edit: []
  delete: []
  cancelOccurrence: []
  restoreOccurrence: []
}>()

const color = computed(() => courseColor(props.block?.course_name ?? ''))
const room = computed(() => props.rooms.find((item) => item.id === props.block?.room_id) ?? null)
const period = computed(() => {
  const block = props.block
  if (!block) return BOARD_PERIODS[0]!
  return BOARD_PERIODS[periodIndexOf(block.start_time)] ?? BOARD_PERIODS[0]!
})
const duration = computed(() => {
  const block = props.block
  if (!block) return 0
  return durationMinutes(block.start_time, block.end_time)
})
const hasRoomConflict = computed(
  () => props.block?.conflicts.some((conflict) => conflict.kind === 'room') ?? false,
)
const editable = computed(() => props.canManage && props.block?.source === 'template')
/** 单日动作：模板块可「本次停课」，单日覆盖块可「恢复本次」。 */
const occurrenceAction = computed<'cancel' | 'restore' | null>(() => {
  const block = props.block
  if (!block || !props.canManage || !props.canAdjustOccurrence) return null
  return block.source === 'exception' ? 'restore' : 'cancel'
})
const scopeHint = computed(() =>
  props.scope === 'occurrence'
    ? '当前作用域为「本次」：B 视图的拖拽/新建默认只调整当天这一节课，不改动周模板。'
    : '当前作用域为「模板」：拖拽/新建会修改周模板，影响整个生效区间。',
)
const deviceText = computed(() => {
  const device = room.value?.device
  if (!device?.serial) return '—'
  if (device.online === null || device.online === undefined) return device.serial
  return `${device.serial} · ${device.online ? '在线' : '离线'}`
})

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && props.open) emit('close')
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <Transition name="drawer">
    <div v-if="open && block" class="absolute inset-0 z-20 overflow-hidden">
      <div class="absolute inset-0 bg-ink/10" @click="emit('close')" />

      <aside
        class="absolute inset-y-0 right-0 flex w-[360px] max-w-full flex-col border-l border-line bg-surface shadow-[-8px_0_24px_rgba(21,23,28,0.08)]"
        role="dialog"
        aria-modal="true"
        aria-label="课程详情"
      >
        <header
          class="flex h-12 shrink-0 items-center justify-between border-b border-line pl-4 pr-2"
        >
          <span class="text-[12.5px] font-semibold text-ink">课程详情</span>
          <button
            type="button"
            aria-label="关闭详情"
            class="flex h-8 w-8 items-center justify-center rounded-md text-faint transition-colors hover:bg-canvas hover:text-ink"
            @click="emit('close')"
          >
            <AppIcon name="x" :size="15" />
          </button>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-4 py-4">
          <div class="flex items-start gap-2.5">
            <span
              class="mt-0.5 min-h-[34px] w-1 self-stretch rounded-sm"
              :style="{ background: color.fg }"
            />
            <div class="min-w-0">
              <p class="text-[15px] font-semibold leading-[21px] text-ink">
                {{ block.course_name }}
              </p>
              <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
                <StateBadge
                  :label="block.source === 'exception' ? '单日覆盖' : '周模板'"
                  :tone="block.source === 'exception' ? 'violet' : 'accent'"
                />
                <StateBadge v-if="block.is_override" label="调课" tone="amber" :dot="false" />
                <StateBadge v-if="!block.conflicts.length" label="无冲突" tone="emerald" />
                <StateBadge
                  v-else
                  :label="`${hasRoomConflict ? '房间冲突' : '教师冲突'} · ${block.conflicts.length}`"
                  :tone="hasRoomConflict ? 'rose' : 'amber'"
                />
              </div>
            </div>
          </div>

          <div class="mt-4 border-t border-line pt-3">
            <p class="mb-1.5 text-[10px] font-medium tracking-wide text-faint">课程信息</p>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">教师</span>
              <span class="font-medium text-ink-soft">{{ block.teacher || '—' }}</span>
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">房间</span>
              <span class="text-right font-medium text-ink-soft"
                >{{ block.room_code }} · {{ block.room_name }}</span
              >
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">区域</span>
              <span class="font-medium text-ink-soft">{{ block.area_name }}</span>
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">容量</span>
              <span class="font-medium text-ink-soft">{{
                room ? `${room.capacity} 座` : '—'
              }}</span>
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">设备</span>
              <span class="truncate text-right font-mono text-[10.5px] text-ink-soft">{{
                deviceText
              }}</span>
            </div>
          </div>

          <div class="mt-3 border-t border-line pt-3">
            <p class="mb-1.5 text-[10px] font-medium tracking-wide text-faint">时间</p>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">日期</span>
              <span class="font-medium text-ink-soft"
                >{{ formatMonthDay(block.date) }} {{ weekdayName(block.day_of_week) }}</span
              >
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">时间</span>
              <span class="font-mono text-[11px] font-medium text-ink-soft"
                >{{ block.start_time }}–{{ block.end_time }}</span
              >
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">时长 / 节次</span>
              <span class="font-medium text-ink-soft"
                >{{ duration }} 分钟 · {{ period.label }}</span
              >
            </div>
            <div class="flex items-baseline justify-between gap-3 py-1 text-[11.5px]">
              <span class="shrink-0 text-faint">课表</span>
              <span class="font-mono text-[10.5px] text-muted">{{
                shortId(block.schedule_id)
              }}</span>
            </div>
          </div>

          <div v-if="block.note" class="mt-3 border-t border-line pt-3">
            <p class="mb-1.5 text-[10px] font-medium tracking-wide text-faint">备注</p>
            <p class="whitespace-pre-wrap text-[11.5px] leading-5 text-ink-soft">
              {{ block.note }}
            </p>
          </div>

          <div v-for="(conflict, index) in block.conflicts" :key="index" class="mt-3">
            <div
              class="rounded-lg border px-2.5 py-2"
              :class="
                conflict.kind === 'room'
                  ? 'border-rose-200 bg-rose-50'
                  : 'border-amber-200 bg-amber-50'
              "
            >
              <p
                class="flex items-center gap-1.5 text-[11.5px] font-semibold"
                :class="conflict.kind === 'room' ? 'text-rose-700' : 'text-amber-700'"
              >
                <AppIcon name="alert" :size="13" />
                {{
                  conflict.kind === 'room'
                    ? '房间冲突 · 同房间时段重叠'
                    : '教师冲突 · 同一教师时段重叠'
                }}
              </p>
              <p class="mt-1 text-[10.5px] leading-4 text-ink-soft">{{ conflict.message }}</p>
            </div>
          </div>

          <p
            class="mt-3 rounded-lg border border-line bg-canvas/50 px-2.5 py-2 text-[10.5px] leading-4 text-muted"
          >
            <template v-if="block.source === 'exception'">
              <b class="font-medium text-ink-soft">单日覆盖</b>：只影响
              {{ formatMonthDay(block.date) }}（{{ weekdayName(block.day_of_week) }}）这一天，
              不改动周模板。点「恢复本次」会删除当天的单日覆盖，把这一天恢复为周模板。
            </template>
            <template v-else>
              <b class="font-medium text-ink-soft">周模板</b>：每周{{
                weekdayName(block.day_of_week)
              }}
              {{ block.start_time }}–{{ block.end_time }} 自动生成排课实例并触发房间设备事件。{{
                scopeHint
              }}
            </template>
          </p>
        </div>

        <footer
          class="flex shrink-0 flex-col gap-2 border-t border-line bg-[#fcfcfd] px-3.5 py-2.5"
        >
          <div v-if="editable" class="flex items-center gap-2">
            <AppButton
              size="sm"
              icon="pencil"
              class="flex-1"
              :disabled="deleting"
              @click="emit('edit')"
            >
              编辑
            </AppButton>
            <AppButton
              size="sm"
              icon="trash"
              variant="danger"
              class="flex-1"
              :loading="deleting"
              @click="emit('delete')"
            >
              删除
            </AppButton>
          </div>
          <div v-if="occurrenceAction" class="flex items-center gap-2">
            <AppButton
              v-if="occurrenceAction === 'cancel'"
              size="sm"
              variant="secondary"
              icon="calendar"
              class="flex-1"
              :disabled="deleting"
              title="只停课当天这一节，其它周与当天其它课程不受影响"
              @click="emit('cancelOccurrence')"
            >
              本次停课
            </AppButton>
            <AppButton
              v-else
              size="sm"
              variant="secondary"
              icon="rotate-ccw"
              class="flex-1"
              :disabled="deleting"
              title="删除当天的单日覆盖，把这一天恢复为周模板"
              @click="emit('restoreOccurrence')"
            >
              恢复本次
            </AppButton>
            <AppButton v-if="!editable" size="sm" class="flex-1" @click="emit('close')">
              关闭
            </AppButton>
          </div>
          <AppButton
            v-if="!editable && !occurrenceAction"
            size="sm"
            class="w-full"
            @click="emit('close')"
          >
            关闭
          </AppButton>
        </footer>
      </aside>
    </div>
  </Transition>
</template>

<style scoped>
.drawer-enter-active,
.drawer-leave-active {
  transition: opacity 0.18s ease;
}

.drawer-enter-active aside,
.drawer-leave-active aside {
  transition: transform 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

.drawer-enter-from,
.drawer-leave-to {
  opacity: 0;
}

.drawer-enter-from aside,
.drawer-leave-to aside {
  transform: translateX(100%);
}
</style>
