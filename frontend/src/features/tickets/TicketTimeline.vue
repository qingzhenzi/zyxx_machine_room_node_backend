<script setup lang="ts">
/**
 * 工单处理时间线：系统写入（建单 / 状态机 / 重复触发）与人工评论分开呈现。
 * 系统条目用中性色调 + 「系统」标记；评论带首字头像与正文气泡。
 */
import type { TicketEvent } from '@/api/schema'
import AppIcon from '@/components/ui/AppIcon.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import { formatFullDateTime } from '@/utils/format'

import {
  describeTicketEvent,
  isSystemEvent,
  ticketEventKindMeta,
  type TicketEventLike,
} from './ticketMeta'

defineProps<{
  events: TicketEvent[]
}>()

function hasPayload(value: unknown): boolean {
  return Boolean(value) && typeof value === 'object' && Object.keys(value as object).length > 0
}

function actorInitial(event: TicketEventLike): string {
  const name = event.actor_name || '系统'
  return name.slice(0, 1)
}

function isComment(event: TicketEvent): boolean {
  return event.kind === 'commented'
}

/** P1：worker 发送 SLA 超时提醒后写入的 reminded 事件（schema 生成前先按字符串比较）。 */
function isReminded(event: TicketEvent): boolean {
  return String(event.kind) === 'reminded'
}

function eventText(event: TicketEvent): string {
  return isReminded(event) ? '超时提醒已发送' : describeTicketEvent(event)
}

function eventIcon(event: TicketEvent): string {
  return isReminded(event) ? 'bell' : ticketEventKindMeta(event.kind).icon
}
</script>

<template>
  <div v-if="events.length" class="px-5 py-1">
    <ol class="relative">
      <li
        v-for="event in events"
        :key="event.id"
        class="relative flex gap-3 border-b border-line/60 py-3.5 last:border-b-0"
      >
        <div class="relative flex w-7 shrink-0 flex-col items-center">
          <span
            v-if="isComment(event)"
            class="flex h-6 w-6 items-center justify-center rounded-full bg-accent-soft text-[10.5px] font-medium text-accent"
            :title="event.actor_name || '系统'"
          >
            {{ actorInitial(event) }}
          </span>
          <span
            v-else
            class="flex h-6 w-6 items-center justify-center rounded-full border bg-canvas"
            :class="
              isReminded(event) ? 'border-amber-200 text-amber-600' : 'border-line text-faint'
            "
          >
            <AppIcon :name="eventIcon(event)" :size="12" />
          </span>
        </div>

        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="text-[11.5px] font-medium text-ink">
              {{ event.actor_name || '系统' }}
            </span>
            <span class="text-[11px] text-muted">{{ eventText(event) }}</span>
            <span
              v-if="isSystemEvent(event)"
              class="rounded border border-line bg-canvas px-1 py-0.5 text-[9.5px] text-faint"
            >
              系统
            </span>
            <span class="ml-auto text-[10.5px] tabular-nums text-faint">
              {{ formatFullDateTime(event.created_at) }}
            </span>
          </div>

          <p
            v-if="isComment(event) && event.note"
            class="mt-2 whitespace-pre-wrap rounded-md border border-line bg-canvas/50 px-3 py-2 text-[11.5px] leading-5 text-ink-soft"
          >
            {{ event.note }}
          </p>
          <p
            v-else-if="event.kind === 'status_changed' && event.note"
            class="mt-1 whitespace-pre-wrap text-[11px] leading-5 text-muted"
          >
            {{ event.note }}
          </p>

          <!-- 常规状态事件不展开 payload，避免时间线噪音；重复触发事件保留证据 JSON。 -->
          <div v-if="event.kind === 'occurred' && hasPayload(event.payload)" class="mt-2">
            <JsonBlock :value="event.payload" label="事件 payload" collapsible max-height="220px" />
          </div>
        </div>
      </li>
    </ol>
  </div>
  <p v-else class="px-5 py-8 text-center text-[11.5px] text-faint">暂无处理记录。</p>
</template>
