<script setup lang="ts">
import { computed } from 'vue'

import { usageTone } from './machineMeta'

const props = withDefaults(
  defineProps<{
    value: number | null | undefined
    label?: string
    showValue?: boolean
    height?: 'sm' | 'md'
  }>(),
  { label: '', showValue: true, height: 'sm' },
)

const percent = computed(() => {
  if (props.value === null || props.value === undefined || Number.isNaN(props.value)) return null
  return Math.min(100, Math.max(0, props.value))
})

const tone = computed(() => usageTone(percent.value))

const toneClass: Record<string, string> = {
  emerald: 'bg-emerald-500',
  amber: 'bg-amber-500',
  rose: 'bg-rose-500',
  accent: 'bg-accent',
}
</script>

<template>
  <div class="flex min-w-0 items-center gap-2">
    <div v-if="label" class="w-[34px] shrink-0 text-[10px] text-faint">
      {{ label }}
    </div>
    <div
      class="min-w-0 flex-1 overflow-hidden rounded-full bg-canvas"
      :class="height === 'md' ? 'h-2' : 'h-1.5'"
      role="progressbar"
      :aria-valuenow="percent ?? 0"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div
        class="h-full rounded-full transition-[width] duration-300"
        :class="toneClass[tone]"
        :style="{ width: `${percent ?? 0}%` }"
      />
    </div>
    <span
      v-if="showValue"
      class="w-[38px] shrink-0 text-right font-mono text-[10.5px] tabular-nums text-muted"
    >
      {{ percent === null ? '—' : `${percent.toFixed(0)}%` }}
    </span>
  </div>
</template>
