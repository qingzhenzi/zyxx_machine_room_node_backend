<script setup lang="ts">
/** 工单分析 KPI 卡：新增 / 已解决 / 未关闭 / 平均处理时长 / 超时。 */
import { computed } from 'vue'

import AppIcon from '@/components/ui/AppIcon.vue'

import { formatMttr, percentage, type TicketTotalsView } from './ticketStats'

const props = withDefaults(
  defineProps<{
    totals: TicketTotalsView
    days: number
    loading?: boolean
  }>(),
  { loading: false },
)

const cards = computed(() => [
  {
    label: '新增工单',
    value: String(props.totals.created),
    unit: '张',
    note: `近 ${props.days} 天创建`,
    icon: 'ticket',
    tone: 'text-accent',
  },
  {
    label: '已解决',
    value: String(props.totals.resolved),
    unit: '张',
    note: `解决率 ${percentage(props.totals.resolved, props.totals.created)}%`,
    icon: 'check-circle',
    tone: 'text-emerald-600',
  },
  {
    label: '未关闭',
    value: String(props.totals.open_now),
    unit: '张',
    note: `已关闭 ${props.totals.closed} 张`,
    icon: 'clock',
    tone: 'text-amber-600',
  },
  {
    label: '平均处理时长',
    value: formatMttr(props.totals.avg_resolve_secs),
    unit: '',
    note:
      props.totals.median_resolve_secs === null
        ? 'MTTR · 中位数暂无'
        : `MTTR · 中位数 ${formatMttr(props.totals.median_resolve_secs)}`,
    icon: 'reports',
    tone: 'text-violet-600',
  },
  {
    label: '超时工单',
    value: String(props.totals.overdue),
    unit: '张',
    note: `占新增 ${percentage(props.totals.overdue, props.totals.created)}%`,
    icon: 'alert',
    tone: 'text-rose-600',
  },
])
</script>

<template>
  <section class="grid grid-cols-5 gap-3" aria-label="工单分析关键指标">
    <article
      v-for="card in cards"
      :key="card.label"
      class="rounded-xl border border-line bg-surface px-4 py-3.5"
    >
      <div class="flex items-start justify-between">
        <p class="text-[11px] font-medium text-muted">{{ card.label }}</p>
        <AppIcon :name="card.icon" :size="13" class="text-faint" />
      </div>
      <p class="mt-2 text-[22px] font-semibold leading-none tracking-[-0.02em] tabular-nums">
        <span
          v-if="props.loading"
          class="inline-block h-[22px] w-12 animate-pulse rounded bg-canvas align-middle"
        />
        <template v-else>
          <span :class="card.tone">{{ card.value }}</span>
          <small v-if="card.unit" class="ml-1 text-[11px] font-normal text-faint">
            {{ card.unit }}
          </small>
        </template>
      </p>
      <p class="mt-3 truncate text-[10.5px] text-faint" :title="card.note">{{ card.note }}</p>
    </article>
  </section>
</template>
