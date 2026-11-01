<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type { NewWeeklySlot, WeeklySlot } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import { useToastStore } from '@/stores/toast'

import type { SlotDraft } from './scheduleForm'
import {
  emptySlotDraft,
  toNewWeeklySlots,
  validateSlotDrafts,
  weeklySlotToDraft,
  WEEKDAY_OPTIONS,
} from './scheduleForm'

const props = withDefaults(
  defineProps<{
    slots: WeeklySlot[]
    loading?: boolean
    saving?: boolean
    canManage?: boolean
    timezone?: string
  }>(),
  { loading: false, saving: false, canManage: false, timezone: '' },
)

const emit = defineEmits<{
  save: [slots: NewWeeklySlot[]]
  reload: []
}>()

const toast = useToastStore()
const rows = ref<SlotDraft[]>([])
const error = ref('')

watch(
  () => props.slots,
  (slots) => {
    rows.value = slots.map(weeklySlotToDraft)
    error.value = ''
  },
  { immediate: true },
)

const description = computed(() =>
  rows.value.length
    ? `共 ${rows.value.length} 个时段${props.timezone ? ` · 时区 ${props.timezone}` : ''}`
    : '按星期配置固定上课时段，保存时全量替换并重建未来实例。',
)

function addRow() {
  const last = rows.value[rows.value.length - 1]
  rows.value.push(emptySlotDraft(last?.day_of_week ?? 1))
  error.value = ''
}

function removeRow(index: number) {
  rows.value.splice(index, 1)
  error.value = ''
}

function reset() {
  rows.value = props.slots.map(weeklySlotToDraft)
  error.value = ''
}

function save() {
  const message = validateSlotDrafts(rows.value)
  if (message) {
    error.value = message
    toast.error('周模板校验未通过', message)
    return
  }
  const slots = toNewWeeklySlots(rows.value)
  const sorted = [...slots].sort(
    (left, right) =>
      left.day_of_week - right.day_of_week || left.start_time.localeCompare(right.start_time),
  )
  error.value = ''
  emit('save', sorted)
}
</script>

<template>
  <PageSection :body-class="''">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">周模板</h2>
          <p class="mt-0.5 text-[11px] text-muted">{{ description }}</p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <IconButton
            icon="refresh"
            label="刷新周模板"
            :loading="loading"
            :disabled="loading || saving"
            @click="emit('reload')"
          />
          <AppButton v-if="canManage" size="sm" icon="rotate-ccw" :disabled="saving" @click="reset">
            还原
          </AppButton>
          <AppButton
            v-if="canManage"
            size="sm"
            icon="plus"
            :disabled="loading || saving"
            @click="addRow"
          >
            添加时段
          </AppButton>
          <AppButton
            v-if="canManage"
            size="sm"
            variant="primary"
            icon="check"
            :loading="saving"
            :disabled="loading || !rows.length"
            @click="save"
          >
            保存模板
          </AppButton>
        </div>
      </div>
    </template>

    <div
      v-if="error"
      class="mx-5 mt-4 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      {{ error }}
    </div>

    <div v-if="loading" class="px-5 py-3">
      <div v-for="index in 3" :key="index" class="mb-2 h-8 animate-pulse rounded bg-line/60" />
    </div>

    <EmptyState
      v-else-if="!rows.length"
      title="还没有周模板"
      description="添加每周固定的上课时段后，系统会据此生成排课实例并触发事件。"
      icon="calendar"
    >
      <template v-if="canManage" #action>
        <AppButton size="sm" icon="plus" @click="addRow">添加第一个时段</AppButton>
      </template>
    </EmptyState>

    <div v-else class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">星期</th>
            <th class="table-th">开始</th>
            <th class="table-th">结束</th>
            <th class="table-th">课程</th>
            <th class="table-th">教师</th>
            <th class="table-th">备注</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="(row, index) in rows"
            :key="index"
            class="border-b border-line/70 last:border-b-0"
          >
            <td class="table-td pl-5">
              <select
                v-model.number="row.day_of_week"
                class="select input-sm w-[92px]"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行星期`"
              >
                <option v-for="day in WEEKDAY_OPTIONS" :key="day.value" :value="day.value">
                  {{ day.label }}
                </option>
              </select>
            </td>
            <td class="table-td">
              <input
                v-model="row.start_time"
                type="time"
                class="input input-sm w-[112px]"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行开始时间`"
              />
            </td>
            <td class="table-td">
              <input
                v-model="row.end_time"
                type="time"
                class="input input-sm w-[112px]"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行结束时间`"
              />
            </td>
            <td class="table-td">
              <input
                v-model="row.course_name"
                class="input input-sm min-w-[140px]"
                placeholder="课程名称"
                maxlength="80"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行课程名称`"
              />
            </td>
            <td class="table-td">
              <input
                v-model="row.teacher"
                class="input input-sm min-w-[96px]"
                placeholder="教师"
                maxlength="80"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行教师`"
              />
            </td>
            <td class="table-td">
              <input
                v-model="row.note"
                class="input input-sm min-w-[120px]"
                placeholder="可选"
                maxlength="200"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 行备注`"
              />
            </td>
            <td class="table-td pr-5 text-right">
              <IconButton
                v-if="canManage"
                icon="trash"
                label="删除时段"
                size="sm"
                :disabled="saving"
                @click="removeRow(index)"
              />
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </PageSection>
</template>
