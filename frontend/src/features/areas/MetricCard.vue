<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export interface MetricCardProps {
  label: string
  value: string
  note?: string
  badge?: string
  tone?: BadgeTone
  icon?: string
  loading?: boolean
}

withDefaults(defineProps<MetricCardProps>(), {
  note: '',
  badge: '',
  tone: 'slate',
  icon: 'spark',
  loading: false,
})

const toneClasses: Record<BadgeTone, string> = {
  emerald: 'bg-emerald-50 text-emerald-700',
  amber: 'bg-amber-50 text-amber-700',
  rose: 'bg-rose-50 text-rose-700',
  sky: 'bg-sky-50 text-sky-700',
  violet: 'bg-violet-50 text-violet-700',
  slate: 'bg-slate-100 text-slate-600',
  accent: 'bg-accent-soft text-accent',
}
</script>

<template>
  <article class="rounded-xl border border-line bg-surface px-5 py-4">
    <div class="flex items-start justify-between">
      <p class="text-[11.5px] font-medium text-muted">{{ label }}</p>
      <span
        class="flex h-6 w-6 items-center justify-center rounded-md border border-line bg-canvas/70 text-faint"
      >
        <AppIcon :name="icon" :size="13" />
      </span>
    </div>

    <p class="mt-2.5 text-[23px] font-semibold leading-none tracking-[-0.02em] tabular-nums">
      <span
        v-if="loading"
        class="inline-block h-[23px] w-14 animate-pulse rounded bg-canvas align-middle"
      />
      <template v-else>{{ value }}</template>
    </p>

    <div class="mt-4 flex items-center justify-between gap-2">
      <span class="truncate text-[11px] text-faint" :title="note">{{ note }}</span>
      <span
        v-if="badge"
        class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium"
        :class="toneClasses[tone]"
      >
        {{ badge }}
      </span>
    </div>
  </article>
</template>
