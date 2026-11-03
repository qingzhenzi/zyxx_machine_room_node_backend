<script setup lang="ts">
/**
 * 工单列表表格：编号 / 标题 / 状态 / 优先级 / 分类 / 对象 / 来源 / 处理人 / 更新时间。
 * 行点击进详情；加载中显示骨架，空态与错误由页面统一处理。
 */
import type { Ticket } from '@/api/schema'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatRelative, shortId } from '@/utils/format'

import { isOverdue } from './ticketStats'
import {
  ticketCategoryMeta,
  ticketPriorityMeta,
  ticketSourceMeta,
  ticketStatusMeta,
} from './ticketMeta'

const props = withDefaults(
  defineProps<{
    tickets: Ticket[]
    loading?: boolean
    /** room_id -> 房间名称；列表接口只返回 room_id，由页面用房间列表补齐展示。 */
    roomNames?: Record<string, string>
  }>(),
  { loading: false, roomNames: () => ({}) },
)

const emit = defineEmits<{
  open: [ticket: Ticket]
}>()

function roomName(ticket: Ticket): string {
  const id = ticket.room_id
  if (!id) return '未关联房间'
  const name = props.roomNames[id]
  if (name) return name
  return shortId(id)
}

function objectLabel(ticket: Ticket): string {
  const parts: string[] = []
  if (ticket.serial) parts.push(ticket.serial)
  if (ticket.device_slot) parts.push(ticket.device_slot)
  return parts.length ? parts.join(' · ') : '未关联设备'
}

function occurrenceLabel(ticket: Ticket): string {
  const count = ticket.occurrence_count ?? 0
  return count > 1 ? `×${count}` : ''
}
</script>

<template>
  <table v-if="props.tickets.length" class="w-full border-collapse text-left">
    <thead>
      <tr class="border-b border-line bg-canvas/50">
        <th class="table-th w-[128px] pl-5">编号</th>
        <th class="table-th min-w-[220px]">标题</th>
        <th class="table-th w-[92px]">状态</th>
        <th class="table-th w-[84px]">优先级</th>
        <th class="table-th w-[100px]">分类</th>
        <th class="table-th w-[168px]">对象（房间 · 设备）</th>
        <th class="table-th w-[84px]">来源</th>
        <th class="table-th w-[108px]">处理人</th>
        <th class="table-th w-[108px] pr-5">更新时间</th>
      </tr>
    </thead>
    <tbody>
      <tr
        v-for="ticket in props.tickets"
        :key="ticket.id"
        class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
        @click="emit('open', ticket)"
      >
        <td class="table-td pl-5">
          <div class="flex items-center gap-1.5">
            <span class="font-mono text-[11px] font-medium text-ink">{{ ticket.code }}</span>
            <span v-if="occurrenceLabel(ticket)" class="text-[10px] text-faint">
              {{ occurrenceLabel(ticket) }}
            </span>
          </div>
        </td>
        <td class="table-td">
          <span class="line-clamp-1 text-[12px] font-medium text-ink" :title="ticket.title">
            {{ ticket.title }}
          </span>
        </td>
        <td class="table-td">
          <StateBadge
            :label="ticketStatusMeta(ticket.status).label"
            :tone="ticketStatusMeta(ticket.status).tone"
          />
        </td>
        <td class="table-td">
          <StateBadge
            :label="ticketPriorityMeta(ticket.priority).label"
            :tone="ticketPriorityMeta(ticket.priority).tone"
            :dot="false"
          />
        </td>
        <td class="table-td">
          <span class="inline-flex items-center gap-1.5 text-[11.5px]">
            <span
              class="h-1.5 w-1.5 shrink-0 rounded-full"
              :class="ticketCategoryMeta(ticket.category).barClass"
            />
            {{ ticketCategoryMeta(ticket.category).label }}
          </span>
        </td>
        <td class="table-td">
          <div class="text-[11.5px] text-ink-soft">{{ roomName(ticket) }}</div>
          <div
            class="mt-0.5 truncate font-mono text-[10px] text-faint"
            :title="objectLabel(ticket)"
          >
            {{ objectLabel(ticket) }}
          </div>
        </td>
        <td class="table-td">
          <StateBadge
            :label="ticketSourceMeta(ticket.source).label"
            :tone="ticketSourceMeta(ticket.source).tone"
            :dot="false"
          />
        </td>
        <td class="table-td">
          <span v-if="ticket.assignee_name" class="text-[11.5px] text-ink-soft">
            {{ ticket.assignee_name }}
          </span>
          <span v-else class="text-[11.5px] text-faint">未分配</span>
        </td>
        <td class="table-td pr-5">
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-faint">{{ formatRelative(ticket.updated_at) }}</span>
            <span
              v-if="isOverdue(ticket)"
              class="rounded bg-rose-50 px-1.5 py-0.5 text-[9.5px] font-medium text-rose-600"
            >
              超时
            </span>
          </div>
        </td>
      </tr>
    </tbody>
  </table>

  <div v-else-if="props.loading" class="space-y-3 px-5 py-6">
    <div v-for="row in 6" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
  </div>
</template>
