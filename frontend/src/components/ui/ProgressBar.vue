<script setup lang="ts">
import { computed } from 'vue'

type Tone = 'accent' | 'emerald' | 'amber' | 'rose' | 'line'

const props = withDefaults(
  defineProps<{
    value: number
    tone?: Tone
    size?: 'xs' | 'sm'
  }>(),
  { tone: 'accent', size: 'xs' },
)

const width = computed(() => Math.min(100, Math.max(0, props.value)))

const toneClass: Record<Tone, string> = {
  accent: 'bg-accent',
  emerald: 'bg-emerald-500',
  amber: 'bg-amber-500',
  rose: 'bg-rose-500',
  line: 'bg-line-strong',
}
</script>

<template>
  <div
    class="overflow-hidden rounded-full bg-line"
    :class="size === 'sm' ? 'h-1.5' : 'h-1'"
    role="progressbar"
    :aria-valuenow="width"
    aria-valuemin="0"
    aria-valuemax="100"
  >
    <div
      class="h-full rounded-full transition-[width] duration-500"
      :class="toneClass[tone]"
      :style="{ width: `${width}%` }"
    />
  </div>
</template>
