<script setup lang="ts">
import { computed } from 'vue'

import type { Room, ScheduleEvent } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { triggerKindLabel, triggerKindTone } from '@/features/events/kinds'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const props = withDefaults(
  defineProps<{
    events: ScheduleEvent[]
    loading: boolean
    error?: string
    rooms?: Room[]
  }>(),
  { error: '', rooms: () => [] },
)

const emit = defineEmits<{
  retry: []
}>()

const roomNames = computed(() => new Map(props.rooms.map((room) => [room.id, room.name])))

function roomLabel(event: ScheduleEvent): string {
  if (!event.room_id) return '—'
  return roomNames.value.get(event.room_id) ?? shortId(event.room_id)
}

function triggeredAt(event: ScheduleEvent): string | undefined {
  return event.triggered_at_local ?? event.triggered_at
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

    <div
      v-if="error && events.length"
      role="alert"
      class="mx-5 mt-4 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <ul v-if="events.length" class="divide-y divide-line/70">
      <li v-for="event in events" :key="event.id" class="flex items-center gap-3 px-5 py-2.5">
        <StateBadge
          :label="triggerKindLabel(event.event_kind)"
          :tone="triggerKindTone(event.event_kind)"
          :dot="false"
        />
        <span class="min-w-0 flex-1 truncate text-[12px] text-ink">
          {{ event.title || '未命名事件' }}
        </span>
        <span v-if="rooms.length" class="w-[104px] shrink-0 truncate text-[11px] text-muted">
          {{ roomLabel(event) }}
        </span>
        <span class="shrink-0 font-mono text-[10px] text-faint">{{ event.timezone ?? '—' }}</span>
        <span
          class="shrink-0 text-[10.5px] text-faint"
          :title="formatFullDateTime(triggeredAt(event))"
        >
          {{ formatRelative(triggeredAt(event)) }}
        </span>
      </li>
    </ul>

    <div v-else-if="loading" class="space-y-3 px-5 py-6">
      <div v-for="row in 4" :key="row" class="h-8 animate-pulse rounded-md bg-canvas" />
    </div>

    <EmptyState
      v-else-if="!error"
      title="还没有触发事件"
      description="当课表规则命中排课实例后，事件会出现在这里。"
      icon="zap"
    />

    <EmptyState v-else title="加载触发事件失败" :description="error" icon="alert">
      <template #action>
        <AppButton icon="refresh" @click="emit('retry')">重试</AppButton>
      </template>
    </EmptyState>
  </section>
</template>
