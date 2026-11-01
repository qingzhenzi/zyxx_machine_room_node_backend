<script setup lang="ts">
import type { PresenceEvent } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatRelative } from '@/utils/format'

defineProps<{
  events: PresenceEvent[]
  loading: boolean
}>()

const emit = defineEmits<{
  open: [serial: string]
}>()
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

    <ul v-if="events.length" class="divide-y divide-line/70">
      <li v-for="event in events" :key="event.id">
        <button
          type="button"
          class="flex w-full items-center gap-3 px-5 py-2.5 text-left transition-colors hover:bg-canvas/50"
          @click="emit('open', event.serial)"
        >
          <StateBadge
            :label="event.current_state === 'online' ? '上线' : '离线'"
            :tone="event.current_state === 'online' ? 'emerald' : 'rose'"
          />
          <span class="w-[92px] shrink-0 font-mono text-[11px] text-ink-soft">
            {{ event.device_slot }}
          </span>
          <span class="min-w-0 flex-1 truncate font-mono text-[10.5px] text-faint">
            {{ event.serial }}
          </span>
          <span class="shrink-0 text-[10.5px] text-faint">
            {{ formatRelative(event.occurred_at) }}
          </span>
        </button>
      </li>
    </ul>

    <EmptyState
      v-else
      title="还没有在线事件"
      description="设备心跳状态翻转后，这里会出现上线/离线记录。"
      icon="wifi"
    />
  </section>
</template>
