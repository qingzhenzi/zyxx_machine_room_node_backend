<script setup lang="ts">
import { ref } from 'vue'

import type { DeviceOnlineState, PresenceEvent } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const props = defineProps<{
  events: PresenceEvent[]
  liveIds: number[]
  loading: boolean
  error: string
  canLoadMore: boolean
}>()

const emit = defineEmits<{
  loadMore: []
  retry: []
}>()

const expandedId = ref<number | null>(null)

function onlineMeta(state: DeviceOnlineState): { label: string; tone: 'emerald' | 'rose' } {
  return state === 'online' ? { label: '在线', tone: 'emerald' } : { label: '离线', tone: 'rose' }
}

function isLive(event: PresenceEvent): boolean {
  return props.liveIds.includes(event.id)
}

function toggle(event: PresenceEvent) {
  expandedId.value = expandedId.value === event.id ? null : event.id
}
</script>

<template>
  <div>
    <div
      v-if="error && events.length"
      role="alert"
      class="mx-5 mt-4 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <table v-if="events.length" class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th pl-5">状态变化</th>
          <th class="table-th">设备</th>
          <th class="table-th">位置</th>
          <th class="table-th">发生时间</th>
          <th class="table-th pr-5 text-right">详情</th>
        </tr>
      </thead>
      <tbody>
        <template v-for="event in events" :key="event.id">
          <tr
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            :class="isLive(event) ? 'bg-accent-soft/40' : ''"
          >
            <td class="table-td pl-5">
              <div class="flex items-center gap-1.5">
                <StateBadge
                  :label="onlineMeta(event.previous_state).label"
                  :tone="onlineMeta(event.previous_state).tone"
                  :dot="false"
                />
                <AppIcon name="arrow-right" :size="11" class="text-faint" />
                <StateBadge
                  :label="onlineMeta(event.current_state).label"
                  :tone="onlineMeta(event.current_state).tone"
                />
                <span
                  v-if="isLive(event)"
                  class="rounded bg-accent-soft px-1.5 py-0.5 text-[9.5px] font-medium text-accent"
                >
                  实时
                </span>
              </div>
            </td>
            <td class="table-td">
              <div class="font-mono text-[11px] text-ink">{{ event.device_slot }}</div>
              <div class="font-mono text-[10px] text-faint" :title="event.serial">
                {{ shortId(event.serial, 16) }}
              </div>
            </td>
            <td class="table-td">
              <div class="text-[11px] text-ink-soft" :title="event.room_id ?? ''">
                房间 {{ shortId(event.room_id) }}
              </div>
              <div class="text-[10px] text-faint" :title="event.area_id ?? ''">
                区域 {{ shortId(event.area_id) }}
              </div>
            </td>
            <td class="table-td">
              <div class="text-[11px] text-ink-soft" :title="formatFullDateTime(event.occurred_at)">
                {{ formatRelative(event.occurred_at) }}
              </div>
              <div class="font-mono text-[10px] text-faint">
                {{ formatFullDateTime(event.occurred_at) }}
              </div>
            </td>
            <td class="table-td pr-5 text-right">
              <IconButton
                :icon="expandedId === event.id ? 'chevron-down' : 'chevron-right'"
                :label="expandedId === event.id ? '收起事件 payload' : '展开事件 payload'"
                size="sm"
                @click="toggle(event)"
              />
            </td>
          </tr>
          <tr v-if="expandedId === event.id" class="border-b border-line/70 last:border-b-0">
            <td colspan="5" class="bg-canvas/30 px-5 py-3">
              <div class="mb-1.5 flex items-center gap-2 text-[10.5px] text-faint">
                <span>事件 #{{ event.id }} payload</span>
                <span class="font-mono">{{ event.occurred_at }}</span>
              </div>
              <JsonBlock :value="event.payload" label="payload" />
            </td>
          </tr>
        </template>
      </tbody>
    </table>

    <div v-else-if="loading" class="space-y-3 px-5 py-6">
      <div v-for="row in 4" :key="row" class="h-10 animate-pulse rounded-md bg-canvas" />
    </div>

    <EmptyState
      v-else-if="!error"
      title="还没有在线事件"
      description="设备上线/离线状态翻转后，历史事件会记录在这里；保持实时流连接可以立即看到新事件。"
      icon="wifi"
    />

    <EmptyState v-else title="加载在线事件失败" :description="error" icon="alert">
      <template #action>
        <AppButton icon="refresh" @click="emit('retry')">重试</AppButton>
      </template>
    </EmptyState>

    <div
      v-if="events.length"
      class="flex items-center justify-between border-t border-line bg-canvas/40 px-5 py-2.5"
    >
      <span class="text-[11px] text-muted">已加载 {{ events.length }} 条事件</span>
      <AppButton v-if="canLoadMore" size="sm" :loading="loading" @click="emit('loadMore')">
        加载更多
      </AppButton>
      <span v-else class="text-[11px] text-faint">已加载全部</span>
    </div>
  </div>
</template>
