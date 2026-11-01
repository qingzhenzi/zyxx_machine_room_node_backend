<script setup lang="ts">
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export interface OverviewStat {
  label: string
  value: string
  note: string
  badge?: string
  tone?: BadgeTone
  icon: string
  loading?: boolean
}

withDefaults(defineProps<{ stat: OverviewStat }>(), {})
</script>

<template>
  <article class="rounded-xl border border-line bg-surface px-5 py-4">
    <div class="flex items-start justify-between">
      <p class="text-[11.5px] font-medium text-muted">{{ stat.label }}</p>
      <span
        class="flex h-6 w-6 items-center justify-center rounded-md border border-line bg-canvas/70 text-faint"
      >
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.7"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <template v-if="stat.icon === 'monitor'">
            <rect x="3" y="4" width="18" height="12" rx="2" />
            <path d="M9 20h6M12 16v4" />
          </template>
          <template v-else-if="stat.icon === 'door'">
            <path d="M5 20V5.5A1.5 1.5 0 0 1 6.5 4H17a2 2 0 0 1 2 2v14" />
            <path d="M3 20h18M14 12h.01" />
          </template>
          <template v-else-if="stat.icon === 'calendar'">
            <rect x="3.5" y="5" width="17" height="15.5" rx="2" />
            <path d="M8 3v4M16 3v4M3.5 10h17" />
          </template>
          <template v-else>
            <path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z" />
            <path d="m4 7.5 8 4.5 8-4.5M12 12v9" />
          </template>
        </svg>
      </span>
    </div>

    <p class="mt-2.5 text-[23px] font-semibold leading-none tracking-[-0.02em] tabular-nums">
      {{ stat.loading ? '—' : stat.value }}
    </p>

    <div class="mt-4 flex items-center justify-between gap-2">
      <span class="truncate text-[11px] text-faint">{{ stat.note }}</span>
      <span
        v-if="stat.badge"
        class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium"
        :class="
          {
            emerald: 'bg-emerald-50 text-emerald-700',
            amber: 'bg-amber-50 text-amber-700',
            rose: 'bg-rose-50 text-rose-700',
            sky: 'bg-sky-50 text-sky-700',
            violet: 'bg-violet-50 text-violet-700',
            slate: 'bg-slate-100 text-slate-600',
            accent: 'bg-accent-soft text-accent',
          }[stat.tone ?? 'slate']
        "
      >
        {{ stat.badge }}
      </span>
    </div>
  </article>
</template>
