import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { api } from '@/api'
import type { PresenceEvent } from '@/api/schema'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

const SEEN_KEY = 'rn:presence:seen-id'

function readSeenId(): number {
  try {
    const raw = localStorage.getItem(SEEN_KEY)
    return raw ? Number(raw) || 0 : 0
  } catch {
    return 0
  }
}

function writeSeenId(value: number) {
  try {
    localStorage.setItem(SEEN_KEY, String(value))
  } catch {
    // 存储不可写时只在内存中记录
  }
}

/**
 * 工作台壳层共享数据：侧边栏摘要、通知中心（设备在线事件）与顶栏状态。
 * 所有请求都是尽力而为：没有权限（403）时静默跳过，不阻塞页面。
 */
export const useShellStore = defineStore('shell', () => {
  const rooms = ref(0)
  const devices = ref(0)
  const onlineDevices = ref(0)
  const presenceEvents = ref<PresenceEvent[]>([])
  const lastSeenId = ref(readSeenId())
  const loaded = ref(false)
  const loading = ref(false)
  const error = ref<string | null>(null)

  let inflight: Promise<void> | null = null

  const unreadCount = computed(
    () => presenceEvents.value.filter((event) => event.id > lastSeenId.value).length,
  )

  async function loadSummary() {
    const auth = useAuthStore()
    const [roomPage, devicePage] = await Promise.allSettled([
      auth.can(PERMISSIONS.roomRead)
        ? api.listRooms({ page_size: 1 })
        : Promise.reject(new Error('skip')),
      auth.can(PERMISSIONS.deviceRead)
        ? api.listDevices({ page_size: 100 })
        : Promise.reject(new Error('skip')),
    ])

    if (roomPage.status === 'fulfilled') rooms.value = roomPage.value.total ?? 0
    if (devicePage.status === 'fulfilled') {
      devices.value = devicePage.value.total ?? 0
      onlineDevices.value = (devicePage.value.items ?? []).filter(
        (device) => device.online === 'online',
      ).length
    }
  }

  async function refreshPresence() {
    const auth = useAuthStore()
    if (!auth.can(PERMISSIONS.deviceRead)) return
    try {
      const data = await api.listDeviceEvents({ limit: 12 })
      presenceEvents.value = data.items
      // 首次加载把当前事件视为已读，避免一登录就出现红点。
      if (lastSeenId.value === 0 && data.items.length) {
        markAllSeen()
      }
    } catch {
      // 无 device:read 权限或 SSE 不可用时静默忽略
    }
  }

  async function bootstrap(force = false) {
    if (loaded.value && !force) return
    if (inflight) return inflight

    inflight = (async () => {
      loading.value = true
      error.value = null
      try {
        await Promise.all([loadSummary(), refreshPresence()])
        loaded.value = true
      } catch (caught) {
        error.value = errorMessage(caught, '工作台数据加载失败')
      } finally {
        loading.value = false
        inflight = null
      }
    })()

    return inflight
  }

  function markAllSeen() {
    if (!presenceEvents.value.length) return
    lastSeenId.value = Math.max(...presenceEvents.value.map((event) => event.id))
    writeSeenId(lastSeenId.value)
  }

  function markSeen(id: number) {
    if (id <= lastSeenId.value) return
    lastSeenId.value = id
    writeSeenId(id)
  }

  return {
    rooms,
    devices,
    onlineDevices,
    presenceEvents,
    lastSeenId,
    loaded,
    loading,
    error,
    unreadCount,
    bootstrap,
    refreshPresence,
    markAllSeen,
    markSeen,
  }
})
