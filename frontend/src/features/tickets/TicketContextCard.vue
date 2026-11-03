<script setup lang="ts">
/**
 * 工单详情右栏「关联上下文」：房间 / 机器节点 / 健康与在线状态 + 现有详情页跳转。
 * 数据来自 TicketDetail.context（契约 §3 TicketContext，字段全部可空）。
 */
import { computed } from 'vue'
import { RouterLink } from 'vue-router'

import type { Ticket, TicketContext } from '@/api/schema'
import AppIcon from '@/components/ui/AppIcon.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { machineHealthMeta } from '@/features/machines/machineMeta'
import { formatRelative } from '@/utils/format'

import { ticketMetadataHealth } from './ticketMeta'

const props = withDefaults(
  defineProps<{
    ticket: Ticket
    context?: TicketContext | null
    loading?: boolean
  }>(),
  { context: null, loading: false },
)

const roomId = computed(() => props.context?.room_id || props.ticket.room_id || '')
const serial = computed(() => props.context?.serial || props.ticket.serial || '')

const hasObject = computed(() => Boolean(roomId.value || serial.value))

const roomTitle = computed(() => {
  if (props.context?.room_name) return props.context.room_name
  return roomId.value ? '已关联房间' : '未关联房间'
})

const roomSubtitle = computed(() => {
  const parts: string[] = []
  if (props.context?.area_name) parts.push(props.context.area_name)
  if (props.context?.room_code) parts.push(props.context.room_code)
  return parts.length ? parts.join(' · ') : '未上报房间信息'
})

const machineTitle = computed(() => serial.value || '未关联机器')

const machineSubtitle = computed(() => {
  const parts: string[] = []
  if (serial.value) parts.push(serial.value)
  const slot = props.context?.device_slot || props.ticket.device_slot
  if (slot) parts.push(slot)
  return parts.length ? parts.join(' · ') : '人工工单可不关联设备'
})

const health = computed(() =>
  machineHealthMeta(props.context?.health_status ?? ticketMetadataHealth(props.ticket.metadata)),
)

const healthScore = computed(() => {
  const score = props.context?.health_score
  return typeof score === 'number' && Number.isFinite(score) ? score : null
})

function onlineMeta(value: boolean | null | undefined): { label: string; tone: BadgeTone } {
  if (value === true) return { label: '在线', tone: 'emerald' }
  if (value === false) return { label: '离线', tone: 'rose' }
  return { label: '未知', tone: 'slate' }
}

const online = computed(() => onlineMeta(props.context?.online))
</script>

<template>
  <div class="px-5 py-4">
    <div v-if="props.loading" class="space-y-3">
      <div class="h-10 animate-pulse rounded-md bg-canvas" />
      <div class="h-10 animate-pulse rounded-md bg-canvas" />
    </div>

    <template v-else>
      <div v-if="hasObject" class="flex flex-col gap-4">
        <div class="flex gap-3">
          <span
            class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-line bg-canvas/70 text-faint"
          >
            <AppIcon name="door" :size="13" />
          </span>
          <div class="min-w-0">
            <p class="text-[10.5px] text-faint">房间</p>
            <p class="mt-0.5 truncate text-[12px] font-medium text-ink">{{ roomTitle }}</p>
            <p class="mt-0.5 truncate text-[10.5px] text-muted">{{ roomSubtitle }}</p>
          </div>
        </div>

        <div class="flex gap-3">
          <span
            class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-line bg-canvas/70 text-faint"
          >
            <AppIcon name="cpu" :size="13" />
          </span>
          <div class="min-w-0">
            <p class="text-[10.5px] text-faint">机器节点</p>
            <p class="mt-0.5 truncate font-mono text-[11.5px] font-medium text-ink">
              {{ machineTitle }}
            </p>
            <p class="mt-0.5 truncate text-[10.5px] text-muted">{{ machineSubtitle }}</p>
            <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
              <span class="flex items-center gap-1.5 text-[10.5px] text-muted">
                机器健康
                <StateBadge :label="health.label" :tone="health.tone" />
                <span v-if="healthScore !== null" class="font-mono text-[10px] text-faint">
                  {{ healthScore }} 分
                </span>
              </span>
              <span class="flex items-center gap-1.5 text-[10.5px] text-muted">
                设备
                <StateBadge :label="online.label" :tone="online.tone" />
              </span>
            </div>
            <p v-if="props.context?.collected_at" class="mt-1.5 text-[10px] text-faint">
              最近上报 {{ formatRelative(props.context.collected_at) }}
            </p>
          </div>
        </div>
      </div>

      <p v-else class="text-[11.5px] text-muted">这张工单没有关联房间或设备，属于纯人工上报。</p>

      <div class="mt-4 flex flex-wrap gap-2 border-t border-line/70 pt-3">
        <RouterLink
          v-if="roomId"
          :to="{ name: 'room-detail', params: { id: roomId } }"
          class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
        >
          房间详情
        </RouterLink>
        <RouterLink
          v-if="serial"
          :to="{ name: 'device-detail', params: { serial } }"
          class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
        >
          设备详情
        </RouterLink>
        <RouterLink
          v-if="serial"
          :to="{ name: 'machine-detail', params: { serial } }"
          class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
        >
          机器详情
        </RouterLink>
        <span v-if="!roomId && !serial" class="text-[10.5px] text-faint">暂无可跳转的关联对象</span>
      </div>
    </template>
  </div>
</template>
