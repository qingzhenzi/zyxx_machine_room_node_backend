<script setup lang="ts">
import { computed } from 'vue'

import type { MachineSnapshotPoint } from '@/api/schema'

const props = defineProps<{
  points: MachineSnapshotPoint[]
  live?: boolean
}>()

const series = [
  { key: 'cpu' as const, label: 'CPU', color: '#d99a2b', fill: 'url(#cpu-fill)' },
  { key: 'memory' as const, label: '内存', color: '#4f8fd6', fill: 'url(#mem-fill)' },
  { key: 'disk' as const, label: '磁盘', color: '#8a7bd8', fill: 'url(#disk-fill)' },
]

const rows = computed(() =>
  [...props.points]
    .sort((a, b) => Date.parse(a.collected_at) - Date.parse(b.collected_at))
    .slice(-60)
    .map((point) => ({
      at: point.collected_at,
      cpu: point.cpu_usage_percent ?? null,
      memory: point.memory_usage_percent ?? null,
      disk: point.disk_usage_percent ?? null,
    })),
)

const W = 760
const H = 220
const PAD = { left: 38, right: 14, top: 14, bottom: 26 }
const plotW = W - PAD.left - PAD.right
const plotH = H - PAD.top - PAD.bottom

function x(index: number): number {
  return PAD.left + (rows.value.length <= 1 ? 0 : (index / (rows.value.length - 1)) * plotW)
}
function y(value: number): number {
  return PAD.top + plotH - (Math.max(0, Math.min(100, value)) / 100) * plotH
}

function linePath(key: 'cpu' | 'memory' | 'disk'): string {
  const valid = rows.value
    .map((row, index) => ({ value: row[key], index }))
    .filter((item): item is { value: number; index: number } => item.value !== null)
  if (!valid.length) return ''
  return valid
    .map((item, position) => `${position === 0 ? 'M' : 'L'}${x(item.index).toFixed(1)},${y(item.value).toFixed(1)}`)
    .join(' ')
}

function areaPath(key: 'cpu' | 'memory' | 'disk'): string {
  const line = linePath(key)
  const valid = rows.value.filter((row) => row[key] !== null)
  if (!line || valid.length < 2) return ''
  const first = valid[0]!
  const last = valid[valid.length - 1]!
  const firstIndex = rows.value.indexOf(first)
  const lastIndex = rows.value.indexOf(last)
  return `${line} L${x(lastIndex).toFixed(1)},${(PAD.top + plotH).toFixed(1)} L${x(firstIndex).toFixed(1)},${(PAD.top + plotH).toFixed(1)} Z`
}

const latest = computed(() => rows.value[rows.value.length - 1] ?? null)
const spanLabel = computed(() => {
  if (rows.value.length < 2) return '等待数据'
  const first = rows.value[0]!
  const last = rows.value[rows.value.length - 1]!
  const seconds = Math.max(0, Math.round((Date.parse(last.at) - Date.parse(first.at)) / 1000))
  return `${rows.value.length} 个采样 · ${seconds}s`
})

function valueLabel(value: number | null | undefined): string {
  return value == null ? '—' : `${value.toFixed(1)}%`
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex flex-wrap items-center justify-between gap-2 border-b border-line px-5 py-3">
      <div class="flex items-center gap-2">
        <span class="flex h-6 w-6 items-center justify-center rounded bg-accent-soft text-accent">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
            <path d="M3 17l5-6 4 3 5-8 4 5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
        <div>
          <h2 class="text-[13px] font-semibold text-ink">实时资源趋势</h2>
          <p class="text-[10px] text-muted">每次快照到达即更新 · {{ spanLabel }}</p>
        </div>
      </div>
      <div class="flex items-center gap-3">
        <span
          class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium"
          :class="live ? 'bg-emerald-50 text-emerald-700' : 'bg-canvas text-faint'"
        >
          <span class="h-1.5 w-1.5 rounded-full" :class="live ? 'bg-emerald-500 animate-pulse' : 'bg-faint'" />
          {{ live ? '实时' : '历史' }}
        </span>
        <span v-for="item in series" :key="item.key" class="inline-flex items-center gap-1.5 text-[10px] text-muted">
          <span class="h-1.5 w-4 rounded-full" :style="{ background: item.color }" />
          {{ item.label }}
          <span class="tabular-nums text-ink">{{ valueLabel(latest?.[item.key]) }}</span>
        </span>
      </div>
    </header>

    <div v-if="rows.length >= 2" class="px-3 py-2">
      <svg :viewBox="`0 0 ${W} ${H}`" class="h-[220px] w-full" role="img" aria-label="CPU、内存、磁盘使用率趋势">
        <defs>
          <linearGradient id="cpu-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#d99a2b" stop-opacity="0.28" />
            <stop offset="100%" stop-color="#d99a2b" stop-opacity="0" />
          </linearGradient>
          <linearGradient id="mem-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#4f8fd6" stop-opacity="0.22" />
            <stop offset="100%" stop-color="#4f8fd6" stop-opacity="0" />
          </linearGradient>
          <linearGradient id="disk-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#8a7bd8" stop-opacity="0.16" />
            <stop offset="100%" stop-color="#8a7bd8" stop-opacity="0" />
          </linearGradient>
        </defs>

        <g class="text-[9px] fill-current text-faint">
          <template v-for="tick in [0, 25, 50, 75, 100]" :key="tick">
            <line
              :x1="PAD.left"
              :x2="W - PAD.right"
              :y1="y(tick)"
              :y2="y(tick)"
              stroke="currentColor"
              stroke-opacity="0.12"
              stroke-dasharray="3 4"
            />
            <text :x="PAD.left - 7" :y="y(tick) + 3" text-anchor="end">{{ tick }}</text>
          </template>
        </g>

        <path v-for="item in series" :key="`${item.key}-area`" :d="areaPath(item.key)" :fill="item.fill" />
        <path
          v-for="item in series"
          :key="`${item.key}-line`"
          :d="linePath(item.key)"
          fill="none"
          :stroke="item.color"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="trend-line"
        />
        <circle
          v-if="latest?.cpu != null"
          :cx="x(rows.length - 1)"
          :cy="y(latest.cpu)"
          r="3"
          fill="#d99a2b"
          class="animate-pulse"
        />
        <circle
          v-if="latest?.memory != null"
          :cx="x(rows.length - 1)"
          :cy="y(latest.memory)"
          r="2.6"
          fill="#4f8fd6"
        />
        <g class="text-[9px] fill-current text-faint">
          <text :x="PAD.left" :y="H - 6">{{ rows[0]?.at?.slice(11, 19) }}</text>
          <text :x="W - PAD.right" :y="H - 6" text-anchor="end">{{ latest?.at?.slice(11, 19) }}</text>
        </g>
      </svg>
    </div>
    <div v-else class="flex h-[140px] items-center justify-center text-[11px] text-faint">
      等待至少两个快照后绘制趋势（当前 {{ rows.length }} 个）
    </div>
  </section>
</template>

<style scoped>
.trend-line {
  transition: d 0.3s ease;
}
</style>
