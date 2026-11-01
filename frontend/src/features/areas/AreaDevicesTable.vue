<script setup lang="ts">
import type { DeviceInfo } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { deviceOnlineMeta, deviceStatusMeta } from '@/features/devices/deviceMeta'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

defineProps<{
  devices: DeviceInfo[]
  loading: boolean
  error: string
}>()

const emit = defineEmits<{
  open: [serial: string]
  retry: []
}>()
</script>

<template>
  <PageSection
    title="设备"
    :description="`该区域下的设备（${devices.length} 台），点击进入设备详情。`"
  >
    <div
      v-if="error && devices.length"
      role="alert"
      class="mx-5 mt-4 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <table v-if="devices.length" class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th pl-5">设备</th>
          <th class="table-th">房间</th>
          <th class="table-th">注册状态</th>
          <th class="table-th">在线状态</th>
          <th class="table-th pr-5">最后心跳</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="device in devices"
          :key="device.serial"
          class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
          @click="emit('open', device.serial)"
        >
          <td class="table-td pl-5">
            <div class="font-mono text-[11px] text-ink" :title="device.serial">
              {{ shortId(device.serial, 16) }}
            </div>
            <div class="mt-0.5 font-mono text-[10.5px] text-faint">{{ device.device_slot }}</div>
          </td>
          <td class="table-td">
            <div class="text-[11.5px] text-ink-soft">{{ device.room_name ?? '未绑定房间' }}</div>
            <div v-if="device.room_code" class="font-mono text-[10px] text-faint">
              {{ device.room_code }}
            </div>
          </td>
          <td class="table-td">
            <StateBadge
              :label="deviceStatusMeta(device.status).label"
              :tone="deviceStatusMeta(device.status).tone"
            />
          </td>
          <td class="table-td">
            <StateBadge
              :label="deviceOnlineMeta(device.online).label"
              :tone="deviceOnlineMeta(device.online).tone"
            />
          </td>
          <td class="table-td pr-5">
            <div
              class="text-[11px] text-ink-soft"
              :title="device.last_seen_at ? formatFullDateTime(device.last_seen_at) : ''"
            >
              {{ formatRelative(device.last_seen_at) }}
            </div>
            <div class="font-mono text-[10px] text-faint">
              {{ formatFullDateTime(device.last_seen_at) }}
            </div>
          </td>
        </tr>
      </tbody>
    </table>

    <div v-else-if="loading" class="space-y-3 px-5 py-6">
      <div v-for="row in 4" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
    </div>

    <EmptyState
      v-else-if="!error"
      title="该区域还没有设备"
      description="设备注册并绑定到区域内的房间后，会在这里汇总展示。"
      icon="monitor"
    />

    <EmptyState v-else title="加载设备失败" :description="error" icon="alert">
      <template #action>
        <AppButton icon="refresh" @click="emit('retry')">重试</AppButton>
      </template>
    </EmptyState>
  </PageSection>
</template>
