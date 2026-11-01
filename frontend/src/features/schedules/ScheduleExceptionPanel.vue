<script setup lang="ts">
import { computed, ref } from 'vue'

import type { ScheduleException, UpsertExceptionRequest } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { formatRelative } from '@/utils/format'

import ScheduleExceptionFormModal from './ScheduleExceptionFormModal.vue'
import { compareDate, weekdayOfDate } from './scheduleForm'

const props = withDefaults(
  defineProps<{
    exceptions: ScheduleException[]
    loading?: boolean
    error?: string
    saving?: boolean
    deleting?: boolean
    canManage?: boolean
  }>(),
  { loading: false, error: '', saving: false, deleting: false, canManage: false },
)

const emit = defineEmits<{
  reload: []
  save: [date: string, input: UpsertExceptionRequest]
  delete: [date: string]
}>()

const modalOpen = ref(false)
const editing = ref<ScheduleException | null>(null)
const deletingDate = ref<string | null>(null)

const sorted = computed(() =>
  [...props.exceptions].sort((left, right) =>
    compareDate(left.exception_date, right.exception_date),
  ),
)

const kindMeta: Record<string, { label: string; tone: BadgeTone }> = {
  cancel: { label: '停课', tone: 'rose' },
  replace: { label: '调课', tone: 'sky' },
}

function kindBadge(exception: ScheduleException) {
  return kindMeta[exception.kind ?? ''] ?? { label: exception.kind ?? '—', tone: 'slate' as const }
}

function slotsSummary(exception: ScheduleException): string {
  if (exception.kind === 'cancel') return '当天全部时段取消'
  const slots = exception.slots ?? []
  if (!slots.length) return '—'
  return slots
    .map((slot) =>
      `${slot.start_time ?? '—'}–${slot.end_time ?? '—'} ${slot.course_name ?? ''}`.trim(),
    )
    .join(' / ')
}

function openCreate() {
  editing.value = null
  modalOpen.value = true
}

function openEdit(exception: ScheduleException) {
  editing.value = exception
  modalOpen.value = true
}

function handleSubmit(payload: { date: string; input: UpsertExceptionRequest }) {
  modalOpen.value = false
  emit('save', payload.date, payload.input)
}

function requestDelete(exception: ScheduleException) {
  if (!exception.exception_date) return
  deletingDate.value = exception.exception_date
}

function confirmDelete() {
  const date = deletingDate.value
  deletingDate.value = null
  if (date) emit('delete', date)
}
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">单日覆盖</h2>
          <p class="mt-0.5 text-[11px] text-muted">
            停课或调课优先于周模板，保存/删除后会自动重建未来排课实例。
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <IconButton
            icon="refresh"
            label="刷新单日覆盖"
            :loading="loading"
            :disabled="loading || saving"
            @click="emit('reload')"
          />
          <AppButton
            v-if="canManage"
            size="sm"
            icon="plus"
            :disabled="loading || saving"
            @click="openCreate"
          >
            新增覆盖
          </AppButton>
        </div>
      </div>
    </template>

    <div
      v-if="error"
      class="mx-5 mt-4 flex items-center justify-between gap-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('reload')">重试</AppButton>
    </div>

    <div v-if="loading" class="px-5 py-3">
      <div v-for="index in 3" :key="index" class="mb-2 h-8 animate-pulse rounded bg-line/60" />
    </div>

    <EmptyState
      v-else-if="!sorted.length"
      title="还没有单日覆盖"
      description="为假期、临时调课等特殊日期添加停课或调课覆盖。"
      icon="calendar"
    >
      <template v-if="canManage" #action>
        <AppButton size="sm" icon="plus" @click="openCreate">新增覆盖</AppButton>
      </template>
    </EmptyState>

    <div v-else class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">日期</th>
            <th class="table-th">类型</th>
            <th class="table-th">覆盖时段</th>
            <th class="table-th">备注</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="exception in sorted"
            :key="exception.exception_date ?? exception.id"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5">
              <span class="font-mono text-[11.5px] text-ink">
                {{ exception.exception_date ?? '—' }}
              </span>
              <span class="ml-1.5 text-[10.5px] text-faint">
                {{ weekdayOfDate(exception.exception_date) }}
              </span>
            </td>
            <td class="table-td">
              <StateBadge :label="kindBadge(exception).label" :tone="kindBadge(exception).tone" />
            </td>
            <td class="table-td max-w-[280px]">
              <span class="line-clamp-2 text-[11.5px]">{{ slotsSummary(exception) }}</span>
            </td>
            <td class="table-td max-w-[200px]">
              <span class="line-clamp-2 text-[11.5px] text-muted">{{ exception.note || '—' }}</span>
            </td>
            <td class="table-td text-[10.5px] text-faint">
              {{ formatRelative(exception.updated_at ?? exception.created_at) }}
            </td>
            <td class="table-td pr-5 text-right">
              <div v-if="canManage" class="flex items-center justify-end gap-1">
                <IconButton
                  icon="pencil"
                  label="编辑单日覆盖"
                  size="sm"
                  :disabled="saving || deleting"
                  @click="openEdit(exception)"
                />
                <IconButton
                  icon="trash"
                  label="删除单日覆盖"
                  size="sm"
                  :disabled="saving || deleting"
                  @click="requestDelete(exception)"
                />
              </div>
              <span v-else class="text-[10.5px] text-faint">只读</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <ScheduleExceptionFormModal
      :open="modalOpen"
      :exception="editing"
      :saving="saving"
      @close="modalOpen = false"
      @submit="handleSubmit"
    />

    <ConfirmDialog
      :open="!!deletingDate"
      title="删除单日覆盖"
      :description="`删除 ${deletingDate ?? ''} 的覆盖设置。`"
      detail="删除后会按周模板重建该日期及未来的排课实例，该操作不可撤销。"
      confirm-label="删除覆盖"
      danger
      :loading="deleting"
      @confirm="confirmDelete"
      @close="deletingDate = null"
    />
  </PageSection>
</template>
