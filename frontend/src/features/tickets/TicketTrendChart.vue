<script setup lang="ts">
/**
 * 新增 vs 关闭 按日分组柱状图（纯 CSS，无图表库）。
 * 每日两组柱：新增（accent）/ 关闭（emerald）；悬停 title 显示当日明细。
 */
import { computed } from 'vue'

import { maxCount, summarizeSeries, type DailySeriesPoint } from './ticketStats'

const props = withDefaults(
  defineProps<{
    series: DailySeriesPoint[]
    loading?: boolean
  }>(),
  { loading: false },
)

const summary = computed(() => summarizeSeries(props.series))
const isEmpty = computed(() => summary.value.created + summary.value.resolved === 0)

const axisMax = computed(() => {
  const max = maxCount(
    props.series.map((point) => ({ count: Math.max(point.created, point.resolved) })),
  )
  return Math.max(2, Math.ceil(max * 1.15))
})

const axisLabels = computed(() => {
  const max = axisMax.value
  return [max, Math.round((max * 2) / 3), Math.round(max / 3), 0]
})

const labelStep = computed(() => {
  const days = props.series.length
  if (days <= 7) return 1
  if (days <= 30) return 5
  return 14
})

function barHeight(value: number): string {
  if (value <= 0) return '0%'
  const percent = (value / axisMax.value) * 100
  return `${Math.max(4, percent).toFixed(1)}%`
}

function dayLabel(point: DailySeriesPoint): string {
  return point.date.slice(5).replace('-', '/')
}

function tooltip(point: DailySeriesPoint): string {
  return `${point.date}：新增 ${point.created} · 关闭 ${point.resolved}`
}
</script>

<template>
  <div class="px-5 py-4">
    <div class="flex items-center justify-end gap-4 text-[10.5px] text-muted">
      <span class="flex items-center gap-1.5">
        <i class="h-2 w-2 rounded-sm bg-accent" aria-hidden="true" />新增
      </span>
      <span class="flex items-center gap-1.5">
        <i class="h-2 w-2 rounded-sm bg-emerald-500" aria-hidden="true" />关闭
      </span>
    </div>

    <div v-if="props.loading" class="mt-3 h-[190px] animate-pulse rounded-md bg-canvas" />

    <div
      v-else-if="isEmpty"
      class="mt-3 flex h-[190px] flex-col items-center justify-center gap-1 text-[11.5px] text-faint"
    >
      <span>所选区间暂无工单数据</span>
      <span class="text-[10.5px]">调整时间范围或等待新的自动工单</span>
    </div>

    <template v-else>
      <div class="mt-3 flex gap-2">
        <div
          class="flex h-[170px] w-7 shrink-0 flex-col justify-between text-right font-mono text-[9px] tabular-nums text-faint"
        >
          <span v-for="label in axisLabels" :key="label">{{ label }}</span>
        </div>
        <div class="relative h-[170px] flex-1 border-b border-line">
          <div
            v-for="(_, index) in 3"
            :key="index"
            class="pointer-events-none absolute inset-x-0 border-t border-dashed border-line/70"
            :style="{ top: `${(index / 3) * 100}%` }"
          />
          <div class="relative flex h-full items-end gap-[1px]">
            <div
              v-for="point in props.series"
              :key="point.date"
              class="flex h-full min-w-[2px] flex-1 items-end justify-center gap-[1px]"
              :title="tooltip(point)"
            >
              <span
                class="w-full rounded-t-sm bg-accent/85 transition-colors hover:bg-accent"
                :style="{
                  maxWidth: props.series.length > 40 ? '4px' : '10px',
                  height: barHeight(point.created),
                }"
              />
              <span
                class="w-full rounded-t-sm bg-emerald-500/80 transition-colors hover:bg-emerald-500"
                :style="{
                  maxWidth: props.series.length > 40 ? '4px' : '10px',
                  height: barHeight(point.resolved),
                }"
              />
            </div>
          </div>
        </div>
      </div>

      <div class="mt-1.5 flex gap-2">
        <div class="w-7 shrink-0" />
        <div class="flex flex-1">
          <span
            v-for="(point, index) in props.series"
            :key="point.date"
            class="flex-1 text-center text-[9px] tabular-nums text-faint"
          >
            {{
              index % labelStep === 0 || index === props.series.length - 1 ? dayLabel(point) : ''
            }}
          </span>
        </div>
      </div>

      <div class="mt-3 flex items-center justify-between text-[10.5px] text-muted">
        <span>
          新增 <b class="text-ink">{{ summary.created }}</b> 张 · 关闭
          <b class="text-ink">{{ summary.resolved }}</b> 张 · 净增
          <b :class="summary.net > 0 ? 'text-rose-600' : 'text-ink'">
            {{ summary.net >= 0 ? '+' : '' }}{{ summary.net }}
          </b>
        </span>
        <span class="text-faint">悬停柱体查看当日明细</span>
      </div>
    </template>
  </div>
</template>
