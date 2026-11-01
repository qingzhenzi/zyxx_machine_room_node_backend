<script setup lang="ts">
import type { Occurrence } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { addDays, toDateInput } from '@/utils/format'

import { wallClock } from './scheduleForm'

withDefaults(
  defineProps<{
    occurrences: Occurrence[]
    loading?: boolean
    error?: string
    from: string
    to: string
    timezone?: string
  }>(),
  { loading: false, error: '', timezone: '' },
)

const emit = defineEmits<{
  'update:from': [value: string]
  'update:to': [value: string]
  query: []
  reload: []
}>()

function presetWeek() {
  const now = new Date()
  emit('update:from', toDateInput(now))
  emit('update:to', toDateInput(addDays(now, 6)))
  emit('query')
}

const sourceMeta: Record<string, { label: string; tone: 'sky' | 'violet' }> = {
  weekly: { label: '周模板', tone: 'sky' },
  custom: { label: '单日覆盖', tone: 'violet' },
}

function sourceBadge(occurrence: Occurrence) {
  return (
    sourceMeta[occurrence.source ?? ''] ?? { label: occurrence.source ?? '—', tone: 'sky' as const }
  )
}
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">排课实例</h2>
          <p class="mt-0.5 text-[11px] text-muted">
            {{ occurrences.length }} 个实例{{ timezone ? ` · 时区 ${timezone}` : '' }} ·
            时间为房间本地时间
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <AppButton size="sm" icon="calendar" :disabled="loading" @click="presetWeek">
            今天起 7 天
          </AppButton>
          <IconButton
            icon="refresh"
            label="刷新排课实例"
            :loading="loading"
            :disabled="loading"
            @click="emit('reload')"
          />
        </div>
      </div>
    </template>

    <div class="flex items-center justify-between gap-3 border-b border-line/70 px-5 py-3">
      <div class="flex items-center gap-2.5">
        <label class="flex items-center gap-1.5 text-[11px] text-muted">
          从
          <input
            :value="from"
            type="date"
            class="input input-sm w-[150px]"
            aria-label="实例开始日期"
            @input="emit('update:from', ($event.target as HTMLInputElement).value)"
          />
        </label>
        <label class="flex items-center gap-1.5 text-[11px] text-muted">
          到
          <input
            :value="to"
            type="date"
            class="input input-sm w-[150px]"
            aria-label="实例结束日期"
            @input="emit('update:to', ($event.target as HTMLInputElement).value)"
          />
        </label>
        <AppButton size="sm" variant="primary" :loading="loading" @click="emit('query')">
          查询
        </AppButton>
      </div>
      <p class="text-[10.5px] text-faint">接口默认范围为今天起 7 天，最长 366 天。</p>
    </div>

    <div
      v-if="error"
      class="mx-5 mt-4 flex items-center justify-between gap-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('reload')">重试</AppButton>
    </div>

    <div v-if="loading" class="px-5 py-3">
      <div v-for="index in 5" :key="index" class="mb-2 h-8 animate-pulse rounded bg-line/60" />
    </div>

    <EmptyState
      v-else-if="!occurrences.length"
      title="所选范围内没有排课实例"
      description="调整日期范围，或先维护周模板 / 单日覆盖并手动重建实例。"
      icon="calendar"
    />

    <div v-else class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">日期</th>
            <th class="table-th">开始</th>
            <th class="table-th">结束</th>
            <th class="table-th">课程</th>
            <th class="table-th">教师</th>
            <th class="table-th">来源</th>
            <th class="table-th pr-5">备注</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="(occurrence, index) in occurrences"
            :key="occurrence.id ?? `${occurrence.occurrence_date}-${index}`"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5 font-mono text-[11.5px] text-ink">
              {{ occurrence.occurrence_date ?? '—' }}
            </td>
            <td class="table-td font-mono text-[11.5px]">
              {{ wallClock(occurrence.start_local) }}
            </td>
            <td class="table-td font-mono text-[11.5px]">{{ wallClock(occurrence.end_local) }}</td>
            <td class="table-td">
              <span class="text-[12px] font-medium text-ink">{{
                occurrence.course_name || '—'
              }}</span>
            </td>
            <td class="table-td text-[11.5px]">{{ occurrence.teacher || '—' }}</td>
            <td class="table-td">
              <StateBadge
                :label="sourceBadge(occurrence).label"
                :tone="sourceBadge(occurrence).tone"
              />
            </td>
            <td class="table-td pr-5 max-w-[220px]">
              <span class="line-clamp-2 text-[11.5px] text-muted">{{
                occurrence.note || '—'
              }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </PageSection>
</template>
