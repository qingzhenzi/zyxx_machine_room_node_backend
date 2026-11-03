<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import AppIcon from '@/components/ui/AppIcon.vue'
import { useInterval } from '@/composables/useInterval'
import { useShellStore } from '@/stores/shell'
import { useTicketTodoStore } from '@/stores/ticketTodo'
import { formatRelative } from '@/utils/format'

const shell = useShellStore()
const tickets = useTicketTodoStore()
const router = useRouter()

const open = ref(false)
const root = ref<HTMLElement | null>(null)

const events = computed(() => shell.presenceEvents)
const eventUnread = computed(() => shell.unreadCount)
const ticketUnread = computed(() => tickets.unreadCount)
const unread = computed(() => eventUnread.value + ticketUnread.value)

function toggle() {
  open.value = !open.value
  if (open.value) {
    void shell.refreshPresence()
    void tickets.refresh()
  }
}

function close() {
  open.value = false
}

function onPointerDown(event: PointerEvent) {
  if (open.value && !root.value?.contains(event.target as Node)) close()
}

async function openEvent(serial: string, eventId: number) {
  shell.markSeen(eventId)
  close()
  await router.push({ name: 'device-detail', params: { serial } })
}

async function openTicket(id: string) {
  const ticket = tickets.todos.find((item) => item.id === id)
  if (ticket) tickets.markSeen(ticket)
  close()
  await router.push({ name: 'ticket-detail', params: { id } })
}

onMounted(() => {
  document.addEventListener('pointerdown', onPointerDown)
  void tickets.refresh()
})
onUnmounted(() => document.removeEventListener('pointerdown', onPointerDown))

// 工单待办用独立低频轮询，不复用 30s 的壳层时钟。
useInterval(() => {
  void tickets.refresh()
}, 60_000)
</script>

<template>
  <div ref="root" class="relative">
    <button
      type="button"
      aria-label="通知"
      class="relative flex h-9 w-9 items-center justify-center rounded-md text-muted transition-colors hover:bg-canvas hover:text-ink"
      @click="toggle"
    >
      <AppIcon name="bell" :size="17" />
      <span
        v-if="unread"
        class="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-accent px-1 text-[9px] font-semibold text-white ring-2 ring-surface"
      >
        {{ unread > 9 ? '9+' : unread }}
      </span>
    </button>

    <div
      v-if="open"
      class="absolute right-0 top-11 z-40 w-[360px] overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-18px_rgba(16,24,40,0.35)]"
    >
      <!-- 我的待办工单 -->
      <div
        v-if="tickets.todos.length"
        class="flex items-center justify-between border-b border-line px-4 py-3"
      >
        <p class="flex items-center gap-1.5 text-[12.5px] font-semibold text-ink">
          我的待办工单
          <span
            v-if="ticketUnread"
            class="rounded-full bg-accent-soft px-1.5 py-px text-[9.5px] font-medium text-accent"
          >
            {{ ticketUnread }}
          </span>
        </p>
        <button
          type="button"
          class="text-[10.5px] text-accent transition-colors hover:text-accent-strong disabled:cursor-not-allowed disabled:text-faint"
          :disabled="!ticketUnread"
          @click="tickets.markAllSeen()"
        >
          全部已读
        </button>
      </div>

      <ul v-if="tickets.todos.length" class="max-h-[240px] overflow-y-auto border-b border-line">
        <li v-for="ticket in tickets.todos" :key="ticket.id">
          <button
            type="button"
            class="flex w-full items-start gap-2.5 px-4 py-3 text-left transition-colors hover:bg-canvas/70"
            @click="openTicket(ticket.id)"
          >
            <span
              class="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full"
              :class="ticket.status === 'open' ? 'bg-amber-500' : 'bg-sky-500'"
            />
            <span class="min-w-0 flex-1">
              <span class="block truncate text-[11.5px] leading-4 font-medium text-ink">
                {{ ticket.title }}
              </span>
              <span class="mt-0.5 block truncate font-mono text-[10.5px] leading-4 text-muted">
                {{ ticket.code }}
              </span>
              <span class="mt-1 block text-[9.5px] text-faint">
                {{ formatRelative(ticket.updated_at) }}
              </span>
            </span>
            <AppIcon name="chevron-right" :size="13" class="mt-1 shrink-0 text-faint" />
          </button>
        </li>
      </ul>

      <!-- 设备在线事件 -->
      <div class="flex items-center justify-between border-b border-line px-4 py-3">
        <p class="text-[12.5px] font-semibold text-ink">设备在线事件</p>
        <button
          type="button"
          class="text-[10.5px] text-accent transition-colors hover:text-accent-strong disabled:cursor-not-allowed disabled:text-faint"
          :disabled="!eventUnread"
          @click="shell.markAllSeen()"
        >
          全部已读
        </button>
      </div>

      <ul v-if="events.length" class="max-h-[280px] overflow-y-auto">
        <li v-for="event in events" :key="event.id">
          <button
            type="button"
            class="flex w-full items-start gap-2.5 border-b border-line/60 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-canvas/70"
            @click="openEvent(event.serial, event.id)"
          >
            <span
              class="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full"
              :class="event.current_state === 'online' ? 'bg-emerald-500' : 'bg-rose-500'"
            />
            <span class="min-w-0 flex-1">
              <span class="block text-[11.5px] leading-4 font-medium text-ink">
                设备 {{ event.current_state === 'online' ? '上线' : '离线' }}
              </span>
              <span class="mt-0.5 block truncate font-mono text-[10.5px] leading-4 text-muted">
                {{ event.device_slot }} · {{ event.serial.slice(0, 8) }}
              </span>
              <span class="mt-1 block text-[9.5px] text-faint">
                {{ formatRelative(event.occurred_at) }}
              </span>
            </span>
            <AppIcon name="chevron-right" :size="13" class="mt-1 shrink-0 text-faint" />
          </button>
        </li>
      </ul>

      <p v-else class="px-4 py-8 text-center text-[11.5px] text-muted">暂无设备在线事件</p>
    </div>
  </div>
</template>
