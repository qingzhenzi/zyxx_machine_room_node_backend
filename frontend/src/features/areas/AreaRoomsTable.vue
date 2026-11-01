<script setup lang="ts">
import type { Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { deviceOnlineMeta } from '@/features/devices/deviceMeta'
import { formatRelative } from '@/utils/format'

defineProps<{
  rooms: Room[]
  loading: boolean
  error: string
}>()

const emit = defineEmits<{
  open: [id: string]
  retry: []
}>()

function slotOf(room: Room): string {
  return room.device?.device_slot ?? room.device_slot ?? '—'
}

function onlineMeta(room: Room) {
  if (room.device?.serial) return deviceOnlineMeta(room.device.online)
  if (room.device_slot) return { label: '待注册', tone: 'amber' as const }
  return { label: '未绑定', tone: 'slate' as const }
}
</script>

<template>
  <PageSection
    title="房间"
    :description="`该区域下的房间（${rooms.length} 个），点击进入房间总览。`"
  >
    <div
      v-if="error && rooms.length"
      role="alert"
      class="mx-5 mt-4 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <table v-if="rooms.length" class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th pl-5">房间</th>
          <th class="table-th">容量</th>
          <th class="table-th">设备槽位</th>
          <th class="table-th">在线状态</th>
          <th class="table-th">更新时间</th>
          <th class="table-th pr-5 text-right">总览</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="room in rooms"
          :key="room.id"
          class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
          :class="room.is_active ? '' : 'opacity-70'"
          @click="emit('open', room.id)"
        >
          <td class="table-td pl-5">
            <div class="flex items-center gap-2">
              <span class="text-[12px] font-medium text-ink">{{ room.name }}</span>
              <StateBadge v-if="!room.is_active" label="已停用" tone="slate" :dot="false" />
            </div>
            <div class="mt-0.5 font-mono text-[10.5px] text-faint">{{ room.code }}</div>
          </td>
          <td class="table-td font-mono text-[11px]">{{ room.capacity }}</td>
          <td class="table-td font-mono text-[11px]">{{ slotOf(room) }}</td>
          <td class="table-td">
            <StateBadge :label="onlineMeta(room).label" :tone="onlineMeta(room).tone" />
          </td>
          <td class="table-td text-[11px] text-faint">
            {{ formatRelative(room.updated_at) }}
          </td>
          <td class="table-td pr-5 text-right">
            <span class="text-[11px] text-accent">查看 →</span>
          </td>
        </tr>
      </tbody>
    </table>

    <div v-else-if="loading" class="space-y-3 px-5 py-6">
      <div v-for="row in 4" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
    </div>

    <EmptyState
      v-else-if="!error"
      title="该区域还没有房间"
      description="房间是课表与设备绑定的最小单位，先在房间页创建或迁移房间到该区域。"
      icon="door"
    />

    <EmptyState v-else title="加载房间失败" :description="error" icon="alert">
      <template #action>
        <AppButton icon="refresh" @click="emit('retry')">重试</AppButton>
      </template>
    </EmptyState>
  </PageSection>
</template>
