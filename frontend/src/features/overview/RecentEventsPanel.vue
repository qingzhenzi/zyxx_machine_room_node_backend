<script setup lang="ts">
import type { ScheduleEvent } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatRelative } from '@/utils/format'

defineProps<{
  events: ScheduleEvent[]
  loading: boolean
}>()

const kindLabels: Record<string, string> = {
  occurrence_start: '课程开始',
  occurrence_end: '课程结束',
  occurrence_start_before: '课前提醒',
  daily_time: '定时触发',
  room_occupied: '房间占用',
  room_idle: '房间空闲',
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">最近课表触发事件</h2>
        <p class="mt-0.5 text-[11px] text-muted">排课实例触发后的投递与重放记录</p>
      </div>
      <RouterLink :to="{ name: 'events' }" class="text-[11px] text-accent hover:underline">
        查看全部
      </RouterLink>
    </header>

    <ul v-if="events.length" class="divide-y divide-line/70">
      <li v-for="event in events" :key="event.id" class="flex items-center gap-3 px-5 py-2.5">
        <StateBadge
          :label="kindLabels[event.event_kind ?? ''] ?? event.event_kind ?? '事件'"
          tone="sky"
          :dot="false"
        />
        <span class="min-w-0 flex-1 truncate text-[12px] text-ink">
          {{ event.title || event.room_id?.slice(0, 8) || '—' }}
        </span>
        <span class="shrink-0 font-mono text-[10px] text-faint">
          {{ event.timezone ?? '—' }}
        </span>
        <span class="shrink-0 text-[10.5px] text-faint">
          {{ formatRelative(event.triggered_at_local ?? event.triggered_at) }}
        </span>
      </li>
    </ul>

    <EmptyState
      v-else
      title="还没有触发事件"
      description="当课表规则命中排课实例后，事件会出现在这里。"
      icon="zap"
    />
  </section>
</template>
