import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { api } from '@/api'
import type { Ticket } from '@/api/schema'
import { useAuthStore } from '@/stores/auth'
import { PERMISSIONS } from '@/utils/permissions'

const SEEN_KEY = 'rn:tickets:seen-at'

function readSeenAt(): string {
  try {
    return localStorage.getItem(SEEN_KEY) ?? ''
  } catch {
    return ''
  }
}

function writeSeenAt(value: string) {
  try {
    localStorage.setItem(SEEN_KEY, value)
  } catch {
    // 存储不可写时只在内存中记录
  }
}

/**
 * 顶栏「我的待办工单」：与设备在线事件的已读机制完全独立（独立 localStorage 键、
 * 独立轮询、失败静默），避免两条通知生命周期互相影响。
 */
export const useTicketTodoStore = defineStore('ticketTodo', () => {
  const todos = ref<Ticket[]>([])
  const loading = ref(false)
  const lastSeenAt = ref(readSeenAt())

  const unreadCount = computed(() => {
    if (!lastSeenAt.value) return todos.value.length
    const seen = Date.parse(lastSeenAt.value)
    if (!Number.isFinite(seen)) return todos.value.length
    return todos.value.filter((ticket) => Date.parse(ticket.updated_at) > seen).length
  })

  async function refresh() {
    const auth = useAuthStore()
    if (!auth.can(PERMISSIONS.ticketRead)) return
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return

    loading.value = true
    try {
      const page = await api.listTickets({ mine: true, active: true, page_size: 10 })
      todos.value = page.items ?? []
      // 首次加载把当前待办视为已读，避免一登录就出现红点。
      if (!lastSeenAt.value && todos.value.length) markAllSeen()
    } catch {
      // 无权限 / 网络失败静默，不影响 presence 通知
    } finally {
      loading.value = false
    }
  }

  function markAllSeen() {
    const newest = todos.value.reduce(
      (max, ticket) => (Date.parse(ticket.updated_at) > max ? Date.parse(ticket.updated_at) : max),
      0,
    )
    const value = new Date(Math.max(Date.now(), newest)).toISOString()
    lastSeenAt.value = value
    writeSeenAt(value)
  }

  function markSeen(ticket: Ticket) {
    const seen = Date.parse(lastSeenAt.value)
    const updated = Date.parse(ticket.updated_at)
    if (!Number.isFinite(updated)) return
    if (Number.isFinite(seen) && updated <= seen) return
    const value = new Date(updated).toISOString()
    lastSeenAt.value = value
    writeSeenAt(value)
  }

  return { todos, loading, lastSeenAt, unreadCount, refresh, markAllSeen, markSeen }
})
