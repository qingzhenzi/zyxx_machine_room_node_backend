<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import AppIcon from '@/components/ui/AppIcon.vue'
import { useShellStore } from '@/stores/shell'
import { formatRelative } from '@/utils/format'

const shell = useShellStore()
const router = useRouter()

const open = ref(false)
const root = ref<HTMLElement | null>(null)

const events = computed(() => shell.presenceEvents)
const unread = computed(() => shell.unreadCount)

function toggle() {
  open.value = !open.value
  if (open.value) void shell.refreshPresence()
}

function close() {
  open.value = false
}

function onPointerDown(event: PointerEvent) {
  if (open.value && !root.value?.contains(event.target as Node)) close()
}

function markAll() {
  shell.markAllSeen()
}

async function openEvent(serial: string, eventId: number) {
  shell.markSeen(eventId)
  close()
  await router.push({ name: 'device-detail', params: { serial } })
}

onMounted(() => document.addEventListener('pointerdown', onPointerDown))
onUnmounted(() => document.removeEventListener('pointerdown', onPointerDown))
</script>

<template>
  <div ref="root" class="relative">
    <button
      type="button"
      aria-label="设备在线通知"
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
      class="absolute right-0 top-11 z-40 w-[340px] overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-18px_rgba(16,24,40,0.35)]"
    >
      <div class="flex items-center justify-between border-b border-line px-4 py-3">
        <p class="text-[12.5px] font-semibold text-ink">设备在线事件</p>
        <button
          type="button"
          class="text-[10.5px] text-accent transition-colors hover:text-accent-strong disabled:cursor-not-allowed disabled:text-faint"
          :disabled="!unread"
          @click="markAll"
        >
          全部已读
        </button>
      </div>

      <ul v-if="events.length" class="max-h-[360px] overflow-y-auto">
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

      <p v-else class="px-4 py-10 text-center text-[11.5px] text-muted">暂无设备在线事件</p>
    </div>
  </div>
</template>
