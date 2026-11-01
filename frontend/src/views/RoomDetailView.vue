<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden, isNotFound } from '@/api'
import type {
  Area,
  Occurrence,
  PresenceEvent,
  Room,
  RoomDevice,
  Schedule,
  ScheduleEvent,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import PresenceFeed from '@/features/areas/PresenceFeed.vue'
import ScheduleEventFeed from '@/features/areas/ScheduleEventFeed.vue'
import RoomDevicePanel from '@/features/rooms/RoomDevicePanel.vue'
import RoomFormModal from '@/features/rooms/RoomFormModal.vue'
import RoomSchedulesPanel from '@/features/rooms/RoomSchedulesPanel.vue'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative, today } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const ACTIVITY_KEYS = ['schedules', 'occurrences', 'presence', 'scheduleEvents'] as const
type ActivityKey = (typeof ACTIVITY_KEYS)[number]

const auth = useAuthStore()
const route = useRoute()
const router = useRouter()

const room = ref<Room | null>(null)
const device = ref<RoomDevice | null>(null)
const areas = ref<Area[]>([])
const loading = ref(false)
const deviceLoading = ref(false)
const error = ref('')
const deviceError = ref('')
const notFound = ref(false)
const formOpen = ref(false)

const schedules = ref<Schedule[]>([])
const todayOccurrences = ref<Occurrence[]>([])
const presenceEvents = ref<PresenceEvent[]>([])
const scheduleEvents = ref<ScheduleEvent[]>([])
const activityLoading = ref(false)
const activityErrors = reactive<Record<ActivityKey, string>>({
  schedules: '',
  occurrences: '',
  presence: '',
  scheduleEvents: '',
})

const roomId = computed(() => String(route.params.id ?? ''))
const timezone = computed(() => room.value?.timezone || 'Asia/Shanghai')
const canManage = computed(() => auth.can(PERMISSIONS.roomManage))
const canManageDevice = computed(() => auth.can(PERMISSIONS.deviceManage))
const canReadSchedules = computed(() => auth.can(PERMISSIONS.scheduleRead))

const boundDevice = computed<RoomDevice | null>(() => device.value ?? room.value?.device ?? null)
const lastSeenAt = computed(() => boundDevice.value?.last_seen_at ?? null)
const deviceOnlineLabel = computed(() => {
  const state = boundDevice.value?.online ?? null
  if (!state) return boundDevice.value?.serial ? '未知' : '未绑定'
  return state === 'online' ? '在线' : '离线'
})
const todayPresenceCount = computed(() => {
  const key = dateKey(new Date().toISOString())
  return presenceEvents.value.filter((event) => dateKey(event.occurred_at) === key).length
})

const infoItems = computed(() => {
  const data = room.value
  if (!data) return []
  return [
    { label: '区域', value: data.area_name },
    { label: '房间编码', value: data.code, mono: true },
    { label: '时区', value: data.timezone, mono: true },
    { label: '容量', value: data.capacity },
    { label: '设备槽位', value: data.device_slot ?? data.device?.device_slot ?? '—', mono: true },
    { label: '状态', value: data.is_active ? '启用' : '已停用' },
    { label: '创建时间', value: formatFullDateTime(data.created_at) },
    { label: '更新时间', value: formatRelative(data.updated_at) },
    { label: '房间 ID', value: data.id, mono: true },
  ]
})

const stats = computed<MetricCardProps[]>(() => {
  const deviceTone = !boundDevice.value?.serial
    ? 'slate'
    : boundDevice.value.online === 'online'
      ? 'emerald'
      : boundDevice.value.online === 'offline'
        ? 'rose'
        : 'slate'
  return [
    {
      label: '设备在线状态',
      value: deviceOnlineLabel.value,
      note: boundDevice.value?.serial
        ? `槽位 ${boundDevice.value.device_slot}`
        : '房间尚未绑定设备',
      tone: deviceTone,
      icon: 'monitor',
      loading: loading.value || deviceLoading.value,
    },
    {
      label: '课表数',
      value: activityErrors.schedules ? '—' : String(schedules.value.length),
      note: '该房间的课表',
      badge: activityErrors.schedules ? '加载失败' : undefined,
      tone: 'sky',
      icon: 'calendar',
      loading: activityLoading.value,
    },
    {
      label: '今日排课实例',
      value: activityErrors.occurrences ? '—' : String(todayOccurrences.value.length),
      note: '按房间时区今天起 1 天',
      badge: activityErrors.occurrences ? '加载失败' : undefined,
      tone: 'violet',
      icon: 'clock',
      loading: activityLoading.value,
    },
    {
      label: '今日设备事件',
      value: activityErrors.presence ? '—' : String(todayPresenceCount.value),
      note: '最近 200 条中今日发生',
      badge: activityErrors.presence ? '加载失败' : `共 ${presenceEvents.value.length} 条`,
      tone: 'emerald',
      icon: 'zap',
      loading: activityLoading.value,
    },
    {
      label: '最近设备心跳',
      value: lastSeenAt.value ? formatRelative(lastSeenAt.value) : '—',
      note: lastSeenAt.value ? formatFullDateTime(lastSeenAt.value) : '设备尚未上报心跳',
      tone: 'slate',
      icon: 'wifi',
      loading: loading.value || deviceLoading.value,
    },
  ]
})

/** 把 UTC 时间换算成房间时区下的 `YYYY-MM-DD`，用于「今日」口径统计。 */
function dateKey(iso: string | null | undefined): string {
  if (!iso) return ''
  try {
    const date = new Date(iso)
    if (Number.isNaN(date.getTime())) return ''
    return new Intl.DateTimeFormat('en-CA', {
      timeZone: timezone.value,
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    }).format(date)
  } catch {
    return iso.slice(0, 10)
  }
}

function activityError(caught: unknown, fallback: string, permission: string): string {
  if (isForbidden(caught)) {
    return `缺少权限：当前账号没有 ${permission} 权限，无法加载该模块。`
  }
  return errorMessage(caught, fallback)
}

async function loadActivity() {
  if (!roomId.value) return
  activityLoading.value = true
  for (const key of ACTIVITY_KEYS) activityErrors[key] = ''

  const results = await Promise.allSettled([
    api.listSchedules({ room_id: roomId.value, page_size: 100 }),
    api.roomOccurrences(roomId.value, { from: today(), to: today() }),
    api.listDeviceEvents({ limit: 200 }),
    api.listScheduleEvents({ room_id: roomId.value, page_size: 20 }),
  ])

  const [scheduleResult, occurrenceResult, presenceResult, eventResult] = results

  if (scheduleResult.status === 'fulfilled') {
    schedules.value = scheduleResult.value.items ?? []
  } else {
    activityErrors.schedules = activityError(
      scheduleResult.reason,
      '加载房间课表失败',
      'schedule:read',
    )
  }

  if (occurrenceResult.status === 'fulfilled') {
    todayOccurrences.value = occurrenceResult.value
  } else {
    activityErrors.occurrences = activityError(
      occurrenceResult.reason,
      '加载今日排课实例失败',
      'schedule:read',
    )
  }

  if (presenceResult.status === 'fulfilled') {
    presenceEvents.value = (presenceResult.value.items ?? [])
      .filter((event) => event.room_id === roomId.value)
      .sort((left, right) => right.occurred_at.localeCompare(left.occurred_at))
      .slice(0, 20)
  } else {
    activityErrors.presence = activityError(
      presenceResult.reason,
      '加载设备在线事件失败',
      'device:read',
    )
  }

  if (eventResult.status === 'fulfilled') {
    scheduleEvents.value = [...(eventResult.value.items ?? [])]
      .sort((left, right) => (right.triggered_at ?? '').localeCompare(left.triggered_at ?? ''))
      .slice(0, 20)
  } else {
    activityErrors.scheduleEvents = activityError(
      eventResult.reason,
      '加载课表触发事件失败',
      'schedule:event:read',
    )
  }

  activityLoading.value = false
}

async function loadDevice() {
  deviceLoading.value = true
  deviceError.value = ''
  try {
    device.value = await api.roomDevice(roomId.value)
  } catch (caught) {
    device.value = null
    deviceError.value = isForbidden(caught)
      ? '缺少权限：当前账号没有设备读取权限（device:read），无法查看房间绑定信息。'
      : errorMessage(caught, '加载房间设备信息失败')
  } finally {
    deviceLoading.value = false
  }
}

async function loadAreas() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 100, include_inactive: true })
    areas.value = data.items ?? []
  } catch {
    areas.value = []
  }
}

async function load() {
  if (!roomId.value) return
  loading.value = true
  error.value = ''
  notFound.value = false
  try {
    room.value = await api.getRoom(roomId.value)
    await Promise.all([loadDevice(), loadActivity()])
  } catch (caught) {
    room.value = null
    device.value = null
    if (isNotFound(caught)) {
      notFound.value = true
    } else if (isForbidden(caught)) {
      error.value = '缺少权限：当前账号没有房间读取权限（room:read）。'
    } else {
      error.value = errorMessage(caught, '加载房间详情失败')
    }
  } finally {
    loading.value = false
  }
}

async function onDeviceChanged() {
  await loadDevice()
  try {
    room.value = await api.getRoom(roomId.value)
  } catch {
    // 刷新房间摘要失败时保留现有数据
  }
}

function openEdit() {
  formOpen.value = true
}

function onSaved(saved: Room) {
  formOpen.value = false
  room.value = saved
  void loadDevice()
  void loadActivity()
}

function backToList() {
  void router.push({ name: 'rooms' })
}

function openDevice(serial: string) {
  void router.push({ name: 'device-detail', params: { serial } })
}

watch(roomId, () => void load())
onMounted(() => {
  void load()
  void loadAreas()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      :title="room?.name ?? '房间总览'"
      :badge="room ? (room.is_active ? '启用' : '已停用') : ''"
    >
      <template #description>
        <RouterLink :to="{ name: 'rooms' }" class="link">← 返回房间列表</RouterLink>
        <span v-if="room">
          · 房间总览 · {{ room.area_name }} · {{ room.code }} · {{ room.timezone }}
        </span>
      </template>
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新房间总览"
          variant="outline"
          :loading="loading || activityLoading"
          @click="load"
        />
        <AppButton
          v-if="room"
          icon="cpu"
          @click="router.push({ name: 'machines', query: { room_id: room.id } })"
        >
          机器总览
        </AppButton>
        <AppButton v-if="canManage && room" icon="pencil" @click="openEdit"> 编辑房间 </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error && room"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div v-if="loading && !room" class="mt-4 grid grid-cols-12 gap-4">
      <div class="col-span-7 h-72 animate-pulse rounded-xl border border-line bg-surface" />
      <div class="col-span-5 h-72 animate-pulse rounded-xl border border-line bg-surface" />
    </div>

    <EmptyState
      v-else-if="notFound"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="房间不存在"
      description="该房间可能已被删除，或链接中的房间 ID 不正确。"
      icon="door"
    >
      <template #action>
        <AppButton icon="arrow-left" @click="backToList">返回房间列表</AppButton>
      </template>
    </EmptyState>

    <EmptyState
      v-else-if="error && !room"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="加载房间详情失败"
      :description="error"
      icon="alert"
    >
      <template #action>
        <AppButton icon="refresh" @click="load">重试</AppButton>
      </template>
    </EmptyState>

    <template v-else-if="room">
      <section class="mt-4 grid grid-cols-5 gap-4" aria-label="房间指标">
        <MetricCard v-for="stat in stats" :key="stat.label" v-bind="stat" />
      </section>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-7">
          <PageSection title="房间信息" description="基础字段、房间级 webhook 与 metadata。">
            <div class="px-5 py-4">
              <InfoList :items="infoItems" :columns="2" />

              <div v-if="room.webhook_url" class="mt-4 border-t border-line/70 pt-3">
                <div class="text-[10.5px] text-faint">房间级 Webhook</div>
                <CopyText :value="room.webhook_url" class="mt-0.5" />
              </div>

              <div class="mt-4 border-t border-line/70 pt-3">
                <JsonBlock :value="room.metadata" collapsible label="metadata（JSON 对象）" />
              </div>
            </div>
          </PageSection>
        </div>

        <div class="col-span-5 flex flex-col gap-4">
          <div
            v-if="deviceError"
            role="alert"
            class="rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-[11.5px] leading-5 text-amber-800"
          >
            {{ deviceError }}
          </div>
          <RoomDevicePanel
            v-else
            :room-id="room.id"
            :device="device"
            :loading="deviceLoading"
            :can-manage="canManageDevice"
            @changed="onDeviceChanged"
          />

          <PageSection title="排障入口" description="查看该房间关联设备的在线事件与全局设备列表。">
            <div class="flex flex-wrap gap-2 px-5 py-4">
              <RouterLink
                :to="{ name: 'device-events' }"
                class="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-3 text-[12px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
              >
                设备在线事件
              </RouterLink>
              <RouterLink
                :to="{ name: 'devices' }"
                class="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-3 text-[12px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
              >
                设备列表
              </RouterLink>
              <span class="inline-flex h-8 items-center text-[11px] text-faint">
                房间 ID：<span class="ml-1 font-mono">{{ room.id }}</span>
              </span>
            </div>
          </PageSection>
        </div>
      </div>

      <RoomSchedulesPanel v-if="canReadSchedules" class="mt-4" :room-id="room.id" />
      <PageSection v-else class="mt-4" title="课表与近期实例">
        <p class="px-5 py-6 text-[11.5px] text-muted">
          当前账号没有课表读取权限（schedule:read），无法查看该房间的课表与排课实例。
        </p>
      </PageSection>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-6">
          <PresenceFeed
            :events="presenceEvents"
            :loading="activityLoading"
            :error="activityErrors.presence"
            @open="openDevice"
            @retry="loadActivity"
          />
        </div>
        <div class="col-span-6">
          <ScheduleEventFeed
            :events="scheduleEvents"
            :loading="activityLoading"
            :error="activityErrors.scheduleEvents"
            @retry="loadActivity"
          />
        </div>
      </div>
    </template>

    <RoomFormModal
      :open="formOpen"
      :room="room"
      :areas="areas"
      @close="formOpen = false"
      @saved="onSaved"
    />
  </div>
</template>
