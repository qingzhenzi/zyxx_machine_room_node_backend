<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden, isNotFound } from '@/api'
import type { Area, DeviceInfo, PresenceEvent, Room, Schedule, ScheduleEvent } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import AreaDevicesTable from '@/features/areas/AreaDevicesTable.vue'
import AreaFormModal from '@/features/areas/AreaFormModal.vue'
import AreaRoomsTable from '@/features/areas/AreaRoomsTable.vue'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import PresenceFeed from '@/features/areas/PresenceFeed.vue'
import ScheduleEventFeed from '@/features/areas/ScheduleEventFeed.vue'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatPercent, formatRelative, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const BLOCK_KEYS = ['rooms', 'devices', 'schedules', 'presence', 'scheduleEvents'] as const
type BlockKey = (typeof BLOCK_KEYS)[number]

const auth = useAuthStore()
const route = useRoute()
const router = useRouter()

const area = ref<Area | null>(null)
const allAreas = ref<Area[]>([])
const rooms = ref<Room[]>([])
const devices = ref<DeviceInfo[]>([])
const schedules = ref<Schedule[]>([])
const presenceEvents = ref<PresenceEvent[]>([])
const scheduleEvents = ref<ScheduleEvent[]>([])
const roomTotal = ref(0)
const deviceTotal = ref(0)
const scheduleTotal = ref(0)
const loading = ref(false)
const error = ref('')
const notFound = ref(false)
const formOpen = ref(false)
const blockErrors = reactive<Record<BlockKey, string>>({
  rooms: '',
  devices: '',
  schedules: '',
  presence: '',
  scheduleEvents: '',
})

const areaId = computed(() => String(route.params.id ?? ''))
const canManage = computed(() => auth.can(PERMISSIONS.areaManage))
const timezone = computed(() => area.value?.timezone || 'Asia/Shanghai')

const parentName = computed(() => {
  const parentId = area.value?.parent_id
  if (!parentId) return '无（顶级区域）'
  return allAreas.value.find((item) => item.id === parentId)?.name ?? shortId(parentId)
})

const roomIds = computed(() => new Set(rooms.value.map((room) => room.id)))
const activeRooms = computed(() => rooms.value.filter((room) => room.is_active).length)
const onlineDevices = computed(
  () => devices.value.filter((device) => device.online === 'online').length,
)

const allAreaSchedules = computed(() =>
  schedules.value.filter((schedule) => roomIds.value.has(schedule.room_id)),
)

const allAreaPresence = computed(() => {
  const target = area.value?.id
  return presenceEvents.value
    .filter((event) => event.area_id === target)
    .sort((left, right) => right.occurred_at.localeCompare(left.occurred_at))
})

const allAreaScheduleEvents = computed(() => {
  const ids = roomIds.value
  return scheduleEvents.value
    .filter((event) => (event.room_id ? ids.has(event.room_id) : false))
    .sort((left, right) => (right.triggered_at ?? '').localeCompare(left.triggered_at ?? ''))
})

const recentPresence = computed(() => allAreaPresence.value.slice(0, 20))
const recentScheduleEvents = computed(() => allAreaScheduleEvents.value.slice(0, 20))

/** 课表事件依赖房间 id 集合，房间加载失败时提示无法筛选而不是误报为空。 */
const scheduleEventFeedError = computed(() => {
  if (blockErrors.scheduleEvents) return blockErrors.scheduleEvents
  if (blockErrors.rooms) return '区域房间数据加载失败，暂时无法按区域筛选课表事件。'
  return ''
})

const presenceTodayCount = computed(() => {
  const today = todayKey()
  return allAreaPresence.value.filter((event) => dateKey(event.occurred_at) === today).length
})

const scheduleTodayCount = computed(() => {
  const today = todayKey()
  return allAreaScheduleEvents.value.filter(
    (event) => dateKey(event.triggered_at_local ?? event.triggered_at) === today,
  ).length
})

const stats = computed<MetricCardProps[]>(() => {
  const onlineRate = deviceTotal.value > 0 ? (onlineDevices.value / deviceTotal.value) * 100 : null
  return [
    {
      label: '房间数',
      value: blockErrors.rooms ? '—' : String(roomTotal.value),
      note: '该区域下的房间',
      badge: `${activeRooms.value} 个启用`,
      tone: 'accent',
      icon: 'door',
      loading: loading.value,
    },
    {
      label: '设备',
      value: blockErrors.devices ? '—' : `${onlineDevices.value} / ${deviceTotal.value}`,
      note: '在线设备 / 设备总数',
      badge: onlineRate === null ? undefined : `在线率 ${formatPercent(onlineRate, 0)}`,
      tone: onlineRate === null ? 'slate' : onlineRate > 0 ? 'emerald' : 'rose',
      icon: 'monitor',
      loading: loading.value,
    },
    {
      label: '今日设备在线事件',
      value: blockErrors.presence ? '—' : String(presenceTodayCount.value),
      note: '最近 200 条中今日发生',
      badge: `共 ${allAreaPresence.value.length} 条`,
      tone: 'sky',
      icon: 'wifi',
      loading: loading.value,
    },
    {
      label: '今日课表事件',
      value:
        blockErrors.scheduleEvents || blockErrors.rooms ? '—' : String(scheduleTodayCount.value),
      note: '最近 100 条中今日发生',
      badge: `共 ${allAreaScheduleEvents.value.length} 条`,
      tone: 'violet',
      icon: 'zap',
      loading: loading.value,
    },
    {
      label: '课表数',
      value:
        blockErrors.schedules || blockErrors.rooms ? '—' : String(allAreaSchedules.value.length),
      note: '该区域房间的课表',
      badge: scheduleTotal.value > schedules.value.length ? '仅前 100 张' : undefined,
      tone: 'slate',
      icon: 'calendar',
      loading: loading.value,
    },
  ]
})

const infoItems = computed(() => {
  const data = area.value
  if (!data) return []
  return [
    { label: '区域编码', value: data.code, mono: true },
    { label: '时区', value: data.timezone, mono: true },
    { label: '父区域', value: parentName.value },
    { label: '状态', value: data.is_active ? '启用' : '已停用' },
    { label: '创建时间', value: formatFullDateTime(data.created_at) },
    { label: '更新时间', value: formatRelative(data.updated_at) },
    { label: '区域 ID', value: data.id, mono: true },
    { label: '描述', value: data.description || '—' },
  ]
})

/** 把 UTC 时间换算成区域时区下的 `YYYY-MM-DD`，用于「今日」口径统计。 */
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

function todayKey(): string {
  return dateKey(new Date().toISOString())
}

function blockError(caught: unknown, fallback: string, permission: string): string {
  if (isForbidden(caught)) {
    return `缺少权限：当前账号没有 ${permission} 权限，无法加载该模块。`
  }
  return errorMessage(caught, fallback)
}

async function load() {
  if (!areaId.value) return
  loading.value = true
  error.value = ''
  notFound.value = false
  for (const key of BLOCK_KEYS) blockErrors[key] = ''

  const results = await Promise.allSettled([
    api.getArea(areaId.value),
    api.listAreas({ page: 1, page_size: 100, include_inactive: true }),
    api.listRooms({ area_id: areaId.value, page_size: 100, include_inactive: true }),
    api.listDevices({ area_id: areaId.value, page_size: 100 }),
    api.listSchedules({ page_size: 100 }),
    api.listDeviceEvents({ limit: 200 }),
    api.listScheduleEvents({ page_size: 100 }),
  ])

  const [
    areaResult,
    areaListResult,
    roomResult,
    deviceResult,
    scheduleResult,
    presenceResult,
    eventResult,
  ] = results

  if (areaResult.status === 'fulfilled') {
    area.value = areaResult.value
  } else {
    area.value = null
    if (isNotFound(areaResult.reason)) {
      notFound.value = true
    } else if (isForbidden(areaResult.reason)) {
      error.value = '缺少权限：当前账号没有区域读取权限（area:read）。'
    } else {
      error.value = errorMessage(areaResult.reason, '加载区域总览失败')
    }
  }

  if (areaListResult.status === 'fulfilled') {
    allAreas.value = areaListResult.value.items ?? []
  } else {
    allAreas.value = []
  }

  if (roomResult.status === 'fulfilled') {
    rooms.value = roomResult.value.items ?? []
    roomTotal.value = roomResult.value.total ?? rooms.value.length
  } else {
    blockErrors.rooms = blockError(roomResult.reason, '加载区域房间失败', 'room:read')
  }

  if (deviceResult.status === 'fulfilled') {
    devices.value = deviceResult.value.items ?? []
    deviceTotal.value = deviceResult.value.total ?? devices.value.length
  } else {
    blockErrors.devices = blockError(deviceResult.reason, '加载区域设备失败', 'device:read')
  }

  if (scheduleResult.status === 'fulfilled') {
    schedules.value = scheduleResult.value.items ?? []
    scheduleTotal.value = scheduleResult.value.total ?? schedules.value.length
  } else {
    blockErrors.schedules = blockError(scheduleResult.reason, '加载课表失败', 'schedule:read')
  }

  if (presenceResult.status === 'fulfilled') {
    presenceEvents.value = presenceResult.value.items ?? []
  } else {
    blockErrors.presence = blockError(presenceResult.reason, '加载设备在线事件失败', 'device:read')
  }

  if (eventResult.status === 'fulfilled') {
    scheduleEvents.value = eventResult.value.items ?? []
  } else {
    blockErrors.scheduleEvents = blockError(
      eventResult.reason,
      '加载课表触发事件失败',
      'schedule:event:read',
    )
  }

  loading.value = false
}

function onSaved(saved: Area) {
  formOpen.value = false
  area.value = saved
  void load()
}

function openRoom(id: string) {
  void router.push({ name: 'room-detail', params: { id } })
}

function openDevice(serial: string) {
  void router.push({ name: 'device-detail', params: { serial } })
}

watch(areaId, () => void load())
onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      :title="area?.name ?? '区域总览'"
      :badge="area ? (area.is_active ? '启用' : '已停用') : ''"
    >
      <template #description>
        <RouterLink :to="{ name: 'areas' }" class="link">← 返回区域列表</RouterLink>
        <span v-if="area">
          · 区域总览 · {{ area.code }} · {{ area.timezone }} ·
          {{ area.webhook_url ? '已配置区域级 webhook' : '未配置区域级 webhook' }}
        </span>
      </template>
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新区域总览"
          variant="outline"
          :loading="loading"
          @click="load"
        />
        <AppButton
          v-if="area"
          icon="cpu"
          @click="router.push({ name: 'machines', query: { area_id: area.id } })"
        >
          机器总览
        </AppButton>
        <AppButton v-if="canManage && area" icon="pencil" @click="formOpen = true">
          编辑区域
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error && area"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div v-if="loading && !area" class="mt-4 grid grid-cols-5 gap-4">
      <div
        v-for="row in 5"
        :key="row"
        class="h-[104px] animate-pulse rounded-xl border border-line bg-surface"
      />
    </div>

    <EmptyState
      v-else-if="notFound"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="区域不存在"
      description="该区域可能已被删除，或链接中的区域 ID 不正确。"
      icon="layers"
    >
      <template #action>
        <AppButton icon="arrow-left" @click="router.push({ name: 'areas' })">
          返回区域列表
        </AppButton>
      </template>
    </EmptyState>

    <EmptyState
      v-else-if="error && !area"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="加载区域总览失败"
      :description="error"
      icon="alert"
    >
      <template #action>
        <AppButton icon="refresh" @click="load">重试</AppButton>
      </template>
    </EmptyState>

    <template v-else-if="area">
      <section class="mt-4 grid grid-cols-5 gap-4" aria-label="区域指标">
        <MetricCard v-for="stat in stats" :key="stat.label" v-bind="stat" />
      </section>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-8 flex flex-col gap-4">
          <AreaRoomsTable
            :rooms="rooms"
            :loading="loading"
            :error="blockErrors.rooms"
            @open="openRoom"
            @retry="load"
          />
          <AreaDevicesTable
            :devices="devices"
            :loading="loading"
            :error="blockErrors.devices"
            @open="openDevice"
            @retry="load"
          />
        </div>

        <div class="col-span-4">
          <PageSection
            title="区域信息"
            description="基础字段、父区域、区域级 webhook 与 metadata。"
          >
            <div class="px-5 py-4">
              <InfoList :items="infoItems" :columns="2" />

              <div class="mt-4 border-t border-line/70 pt-3">
                <div class="text-[10.5px] text-faint">区域级 Webhook</div>
                <CopyText v-if="area.webhook_url" :value="area.webhook_url" class="mt-0.5" />
                <p v-else class="mt-0.5 text-[11.5px] text-muted">
                  未配置（区域级上下线事件不会推送）
                </p>
              </div>

              <div class="mt-4 border-t border-line/70 pt-3">
                <JsonBlock :value="area.metadata" collapsible label="metadata（JSON 对象）" />
              </div>
            </div>
          </PageSection>
        </div>
      </div>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-6">
          <PresenceFeed
            :events="recentPresence"
            :loading="loading"
            :error="blockErrors.presence"
            :rooms="rooms"
            @open="openDevice"
            @retry="load"
          />
        </div>
        <div class="col-span-6">
          <ScheduleEventFeed
            :events="recentScheduleEvents"
            :loading="loading"
            :error="scheduleEventFeedError"
            :rooms="rooms"
            @retry="load"
          />
        </div>
      </div>
    </template>

    <AreaFormModal
      :open="formOpen"
      :area="area"
      :areas="allAreas"
      @close="formOpen = false"
      @saved="onSaved"
    />
  </div>
</template>
