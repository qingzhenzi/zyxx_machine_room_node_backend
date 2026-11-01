<script setup lang="ts">
import { computed } from 'vue'

import type { PresenceEvent, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const props = withDefaults(
  defineProps<{
    events: PresenceEvent[]
    loading: boolean
    error?: string
    rooms?: Room[]
  }>(),
  { error: '', rooms: () => [] },
)

const emit = defineEmits<{
  retry: []
  open: [serial: string]
}>()

const roomNames = computed(() => new Map(props.rooms.map((room) => [room.id, room.name])))

function stateMeta(state: PresenceEvent['current_state']): {
  label: string
  tone: 'emerald' | 'rose'
} {
  return state === 'online' ? { label: '在线', tone: 'emerald' } : { label: '离线', tone: 'rose' }
}

function roomLabel(event: PresenceEvent): string {
  if (!event.room_id) return '未绑定房间'
  return roomNames.value.get(event.room_id) ?? shortId(event.room_id)
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">最近设备在线事件</h2>
        <p class="mt-0.5 text-[11px] text-muted">设备上下线翻转记录，点击查看设备详情</p>
      </div>
      <RouterLink :to="{ name: 'device-events' }" class="text-[11px] text-accent hover:underline">
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
      <li v-for="event in events" :key="event.id">
        <button
          type="button"
          class="flex w-full items-center gap-3 px-5 py-2.5 text-left transition-colors hover:bg-canvas/50"
          @click="emit('open', event.serial)"
        >
          <span class="flex shrink-0 items-center gap-1.5">
            <StateBadge
              :label="stateMeta(event.previous_state).label"
              :tone="stateMeta(event.previous_state).tone"
              :dot="false"
            />
            <AppIcon name="arrow-right" :size="11" class="text-faint" />
            <StateBadge
              :label="stateMeta(event.current_state).label"
              :tone="stateMeta(event.current_state).tone"
            />
          </span>
          <span class="w-[92px] shrink-0 font-mono text-[11px] text-ink-soft">
            {{ event.device_slot }}
          </span>
          <span class="min-w-0 flex-1 truncate font-mono text-[10.5px] text-faint">
            {{ shortId(event.serial, 16) }}
          </span>
          <span v-if="rooms.length" class="w-[104px] shrink-0 truncate text-[11px] text-muted">
            {{ roomLabel(event) }}
          </span>
          <span
            class="shrink-0 text-[10.5px] text-faint"
            :title="formatFullDateTime(event.occurred_at)"
          >
            {{ formatRelative(event.occurred_at) }}
          </span>
        </button>
      </li>
    </ul>

    <div v-else-if="loading" class="space-y-3 px-5 py-6">
      <div v-for="row in 4" :key="row" class="h-8 animate-pulse rounded-md bg-canvas" />
    </div>

    <EmptyState
      v-else-if="!error"
      title="还没有在线事件"
      description="设备心跳状态翻转后，这里会出现上线/离线记录。"
      icon="wifi"
    />

    <EmptyState v-else title="加载在线事件失败" :description="error" icon="alert">
      <template #action>
        <AppButton icon="refresh" @click="emit('retry')">重试</AppButton>
      </template>
    </EmptyState>
  </section>
</template>
