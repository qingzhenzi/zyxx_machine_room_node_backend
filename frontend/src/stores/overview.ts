import { defineStore } from 'pinia'
import { ref } from 'vue'

import { api } from '@/api'
import type { Announcement, AuditLog, PresenceEvent, ScheduleEvent, Transfer } from '@/api/schema'
import type { KpiSummary } from '@/api/types'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

export interface OverviewData {
  kpi: KpiSummary
  todayEvents: number
  recentPresence: PresenceEvent[]
  recentEvents: ScheduleEvent[]
  recentAudit: AuditLog[]
  transfers: Transfer[]
  announcements: Announcement[]
  updatedAt: string
}

function startOfToday(): string {
  const date = new Date()
  date.setHours(0, 0, 0, 0)
  return date.toISOString()
}

function startOfTomorrow(): string {
  const date = new Date()
  date.setHours(24, 0, 0, 0)
  return date.toISOString()
}

export const useOverviewStore = defineStore('overview', () => {
  const data = ref<OverviewData | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  let requestToken = 0

  async function load() {
    const auth = useAuthStore()
    const token = ++requestToken
    loading.value = true
    error.value = null

    const optional = <T>(enabled: boolean, request: Promise<T>) =>
      enabled ? request : Promise.resolve(null)

    try {
      const [
        areas,
        rooms,
        devices,
        schedules,
        images,
        publishedImages,
        todayEvents,
        presence,
        events,
        audit,
        transfers,
        announcements,
      ] = await Promise.all([
        api.listAreas({ page_size: 1 }),
        api.listRooms({ page_size: 1 }),
        api.listDevices({ page_size: 100 }),
        api.listSchedules({ page_size: 1 }),
        api.listImages({ page_size: 1 }),
        auth.can(PERMISSIONS.imageRead)
          ? api.listImages({ status: 'published', page_size: 1 })
          : null,
        api.listScheduleEvents({ from: startOfToday(), to: startOfTomorrow(), page_size: 1 }),
        api.listDeviceEvents({ limit: 10 }),
        api.listScheduleEvents({ page_size: 8 }),
        optional(auth.can(PERMISSIONS.auditRead), api.listAuditLogs({ page_size: 8 })),
        optional(auth.can(PERMISSIONS.storageRead), api.listTransfers({ page_size: 8 })),
        optional(
          auth.can(PERMISSIONS.announcementRead),
          api.listAdminAnnouncements({ page_size: 5 }),
        ),
      ])

      const deviceItems = devices.items ?? []
      const kpi: KpiSummary = {
        areas: areas.total ?? 0,
        rooms: rooms.total ?? 0,
        devices: devices.total ?? 0,
        onlineDevices: deviceItems.filter((device) => device.online === 'online').length,
        schedules: schedules.total ?? 0,
        images: images.total ?? 0,
        publishedImages: publishedImages?.total ?? 0,
        announcements: announcements?.total ?? 0,
      }

      if (token !== requestToken) return

      data.value = {
        kpi,
        todayEvents: todayEvents.total ?? 0,
        recentPresence: presence.items,
        recentEvents: events.items ?? [],
        recentAudit: audit?.items ?? [],
        transfers: (transfers?.items ?? []).filter(
          (transfer) => transfer.status !== 'completed' && transfer.status !== 'expired',
        ),
        announcements: announcements?.items ?? [],
        updatedAt: new Date().toISOString(),
      }
    } catch (caught) {
      if (token === requestToken) error.value = errorMessage(caught, '总览数据加载失败')
    } finally {
      if (token === requestToken) loading.value = false
    }
  }

  return { data, loading, error, load }
})
