<script setup lang="ts">
import { computed } from 'vue'

export type BadgeTone = 'emerald' | 'amber' | 'rose' | 'slate' | 'sky' | 'violet' | 'accent'

const props = withDefaults(
  defineProps<{
    label: string
    tone?: BadgeTone
    dot?: boolean
    pulse?: boolean
  }>(),
  { tone: 'slate', dot: true, pulse: false },
)

const tones: Record<BadgeTone, { badge: string; dot: string }> = {
  emerald: { badge: 'bg-emerald-50 text-emerald-700', dot: 'bg-emerald-500' },
  amber: { badge: 'bg-amber-50 text-amber-700', dot: 'bg-amber-500' },
  rose: { badge: 'bg-rose-50 text-rose-700', dot: 'bg-rose-500' },
  slate: { badge: 'bg-slate-100 text-slate-600', dot: 'bg-slate-400' },
  sky: { badge: 'bg-sky-50 text-sky-700', dot: 'bg-sky-500' },
  violet: { badge: 'bg-violet-50 text-violet-700', dot: 'bg-violet-500' },
  accent: { badge: 'bg-accent-soft text-accent', dot: 'bg-accent' },
}

const current = computed(() => tones[props.tone])
</script>

<template>
  <span
    class="inline-flex items-center gap-1.5 whitespace-nowrap rounded px-1.5 py-0.5 text-[10.5px] font-medium"
    :class="current.badge"
  >
    <span
      v-if="dot"
      class="h-1.5 w-1.5 shrink-0 rounded-full"
      :class="[current.dot, pulse ? 'animate-pulse' : '']"
    />
    {{ label }}
  </span>
</template>
