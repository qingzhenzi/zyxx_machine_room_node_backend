<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api } from '@/api'
import type {
  Area,
  MachineReportSummary,
  MachineSnapshotPoint,
  MachineSummary,
  PresenceEvent,
  Room,
} from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useInterval } from '@/composables/useInterval'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import { MACHINE_HEALTH_FILTERS, machineHealthMeta } from '@/features/machines/machineMeta'
import { useMachineStream, type MachineStreamStatus } from '@/features/machines/useMachineStream'
import { usePresenceStream } from '@/features/machines/usePresenceStream'
import { useToastStore } from '@/stores/toast'
import UsageBar from '@/features/machines/UsageBar.vue'
import { errorMessage } from '@/utils/errors'
import { formatRelative, formatSeconds, shortId } from '@/utils/format'

const router = useRouter()
const route = useRoute()

const PAGE_SIZE = 20
const REFRESH_STORAGE_KEY = 'qe.machines.refresh'
const SNAPSHOT_RECONCILE_DEBOUNCE_MS = 400
const PRESENCE_TOAST_GRACE_MS = 3_000
const PRESENCE_TOAST_DEDUPE_MS = 5_000
const RECENT_CHANGE_TTL_MS = 60_000

type RefreshMode = 'live' | '5s' | '15s' | '30s' | 'paused'

const REFRESH_OPTIONS: { value: RefreshMode; label: string }[] = [
  { value: 'live', label: '实时' },
  { value: '5s', label: '5 秒' },
  { value: '15s', label: '15 秒' },
  { value: '30s', label: '30 秒' },
  { value: 'paused', label: '暂停' },
]

/** `live` 仍保留 15s 全量对齐（注册/删除等非快照变更靠它兜底），其余为显式轮询。 */
const REFRESH_INTERVALS: Record<RefreshMode, number | null> = {
  live: 15_000,
  '5s': 5_000,
  '15s': 15_000,
  '30s': 30_000,
  paused: null,
}

function isRefreshMode(value: string): value is RefreshMode {
  return REFRESH_OPTIONS.some((option) => option.value === value)
}

function readRefreshMode(): RefreshMode {
  try {
    const stored = localStorage.getItem(REFRESH_STORAGE_KEY)
    if (stored && isRefreshMode(stored)) return stored
  } catch {
    // localStorage 不可用时退回默认值
  }
  return 'live'
}

interface RecentChange {
  state: 'online' | 'offline'
  at: number
}

const items = ref<MachineSummary[]>([])
const total = ref(0)
const page = ref(1)
const loading = ref(false)
const error = ref('')

const areas = ref<Area[]>([])
const rooms = ref<Room[]>([])

const keyword = ref('')
const areaId = ref('')
const roomId = ref('')
const health = ref('')
const machineStateFilter = ref('')

const stats = ref({ total: 0, online: 0, alerts: 0, missing: 0, stale: 0, offline: 0 })
const toast = useToastStore()
/** 已见过的 serial：轮询发现新设备时提示（首次加载不提示）。 */
let knownSerials: Set<string> | null = null
/** 未知 serial 只触发一次对齐刷新，避免新机器持续上报时反复全量拉取。 */
const reconciledSnapshotSerials = new Set<string>()
const statsLoading = ref(false)

const refreshMode = ref<RefreshMode>(readRefreshMode())
const refreshTimer = ref<ReturnType<typeof setInterval> | null>(null)

const recentChanges = ref<Record<string, RecentChange>>({})
const presenceToastAt = new Map<string, number>()
const mountedAt = Date.now()
let unmounted = false

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

const visibleRooms = computed(() =>
  areaId.value ? rooms.value.filter((room) => room.area_id === areaId.value) : rooms.value,
)

function machineStateMeta(machine: MachineSummary): { label: string; tone: BadgeTone } {
  if (!machine.report) return { label: '未上报', tone: 'slate' }
  if (machine.online === 'online') {
    return machine.agent_online
      ? { label: '在线', tone: 'emerald' }
      : { label: '在线·遥测过期', tone: 'amber' }
  }
  if (machine.online === 'offline') return { label: '离线', tone: 'rose' }
  return { label: '未知', tone: 'slate' }
}

function recentChangeFor(serial: string): RecentChange | undefined {
  return recentChanges.value[serial]
}

function recentChangeLabel(serial: string): string {
  return recentChangeFor(serial)?.state === 'offline' ? '刚刚离线' : '刚刚上线'
}

function recentChangeClasses(serial: string): string {
  return recentChangeFor(serial)?.state === 'offline'
    ? 'bg-rose-50 text-rose-600'
    : 'bg-emerald-50 text-emerald-600'
}

function rowToneClasses(serial: string): Record<string, boolean> {
  const change = recentChangeFor(serial)
  return {
    'bg-rose-50/60': change?.state === 'offline',
    'bg-emerald-50/60': change?.state === 'online',
  }
}

/** 60s 后清掉「刚刚上线/离线」标记；由 1s ticker 驱动。 */
function pruneRecentChanges(now: number) {
  const entries = Object.entries(recentChanges.value)
  const kept = entries.filter(([, change]) => now - change.at < RECENT_CHANGE_TTL_MS)
  if (kept.length === entries.length) return
  recentChanges.value = Object.fromEntries(kept)
}

const statCards = computed<MetricCardProps[]>(() => [
  {
    label: '机器总数',
    value: String(stats.value.total),
    note: '已注册设备（device_enrollment）',
    badge: `${items.value.length} 条当前页`,
    tone: 'accent',
    icon: 'cpu',
    loading: statsLoading.value,
  },
  {
    label: 'Agent 在线',
    value: String(stats.value.online),
    note: `新鲜快照 · 过期 ${stats.value.stale} · 离线 ${stats.value.offline}（窗口自适应）`,
    badge: stats.value.total
      ? `${Math.round((stats.value.online / stats.value.total) * 100)}%`
      : undefined,
    tone: stats.value.online ? 'emerald' : 'slate',
    icon: 'wifi',
    loading: statsLoading.value,
  },
  {
    label: '健康告警',
    value: String(stats.value.alerts),
    note: '最新快照为 warning / critical',
    badge: stats.value.alerts ? '需关注' : '正常',
    tone: stats.value.alerts ? 'amber' : 'emerald',
    icon: 'alert',
    loading: statsLoading.value,
  },
  {
    label: '未上报',
    value: String(stats.value.missing),
    note: '从未收到 room-agent 快照',
    badge: stats.value.missing ? '待接入' : '全部已接入',
    tone: stats.value.missing ? 'slate' : 'emerald',
    icon: 'sliders',
    loading: statsLoading.value,
  },
])

async function loadStats() {
  statsLoading.value = true
  try {
    // 服务端聚合：不受 page_size=100 限制，与筛选无关（全量机房 KPI）。
    const data = await api.machineSummary()
    stats.value = {
      total: data.total,
      online: data.online,
      alerts: data.alerts,
      missing: data.missing,
      stale: data.stale,
      offline: data.offline,
    }
  } catch {
    // 统计失败不阻断列表
  } finally {
    statsLoading.value = false
  }
}

async function loadOptions() {
  const [areaResult, roomResult] = await Promise.allSettled([
    api.listAreas({ page_size: 100 }),
    api.listRooms({ page_size: 100 }),
  ])
  if (areaResult.status === 'fulfilled') areas.value = areaResult.value.items ?? []
  if (roomResult.status === 'fulfilled') rooms.value = roomResult.value.items ?? []
}

async function load(reset = false) {
  if (reset) page.value = 1
  loading.value = true
  error.value = ''
  try {
    const data = await api.listMachines({
      page: page.value,
      page_size: PAGE_SIZE,
      area_id: areaId.value || undefined,
      room_id: roomId.value || undefined,
      health: health.value || undefined,
      state: machineStateFilter.value || undefined,
      keyword: keyword.value.trim() || undefined,
    })
    items.value = data.items
    total.value = data.total
  } catch (caught) {
    error.value = errorMessage(caught, '机器列表加载失败')
  } finally {
    loading.value = false
  }
}

const { run: searchDebounced } = useDebounceFn(() => void load(true), 300)

watch(keyword, () => searchDebounced())
watch([areaId, roomId], () => {
  if (areaId.value && roomId.value) {
    const room = rooms.value.find((item) => item.id === roomId.value)
    if (room && room.area_id !== areaId.value) roomId.value = ''
  }
  void router.replace({
    query: {
      ...route.query,
      area_id: areaId.value || undefined,
      room_id: roomId.value || undefined,
    },
  })
})

watch(health, () => void load(true))
watch(machineStateFilter, () => void load(true))

function resetFilters() {
  keyword.value = ''
  areaId.value = ''
  roomId.value = ''
  health.value = ''
  machineStateFilter.value = ''
}

function openMachine(serial: string) {
  void router.push({ name: 'machine-detail', params: { serial } })
}

function changePage(next: number) {
  page.value = next
  void load()
}

function refresh() {
  void Promise.all([load(), loadStats()])
}

const { run: refreshFromStream } = useDebounceFn(() => {
  void load()
  void loadStats()
}, SNAPSHOT_RECONCILE_DEBOUNCE_MS)

// 兜底对齐：新设备注册后 presence 事件默认不通知 online、首个遥测快照也可能延迟。
// 实时/轮询模式只是周期不同；暂停时完全不自动拉取。
function stopRefreshTimer() {
  if (refreshTimer.value !== null) {
    clearInterval(refreshTimer.value)
    refreshTimer.value = null
  }
}

function startRefreshTimer() {
  stopRefreshTimer()
  const intervalMs = REFRESH_INTERVALS[refreshMode.value]
  if (intervalMs === null) return
  refreshTimer.value = setInterval(() => {
    if (document.visibilityState === 'hidden') return
    void load()
    void loadStats()
  }, intervalMs)
}

watch(refreshMode, (mode) => {
  try {
    localStorage.setItem(REFRESH_STORAGE_KEY, mode)
  } catch {
    // localStorage 不可用时只影响持久化
  }
  startRefreshTimer()
})

watch(
  () => items.value.map((item) => item.serial),
  (serials) => {
    const current = new Set(serials)
    if (knownSerials && !keyword.value && !areaId.value && !roomId.value && !health.value) {
      for (const machine of items.value) {
        if (!knownSerials.has(machine.serial)) {
          toast.success(
            machine.report ? '新机器已上线' : '新机器已注册，等待首帧遥测',
            `${machine.device_slot} · ${shortId(machine.serial, 10)}`,
          )
        }
      }
    }
    knownSerials = new Set([...(knownSerials ?? []), ...current])
  },
)

function snapshotReport(snapshot: MachineSnapshotPoint): MachineReportSummary {
  return {
    agent_version: snapshot.agent_version,
    collected_at: snapshot.collected_at,
    received_at: snapshot.received_at,
    hostname: snapshot.hostname,
    os_name: snapshot.os_name ?? null,
    kernel_version: snapshot.kernel_version ?? null,
    health_status: snapshot.health_status,
    health_score: snapshot.health_score ?? null,
    cpu_usage_percent: snapshot.cpu_usage_percent ?? null,
    memory_usage_percent: snapshot.memory_usage_percent ?? null,
    disk_usage_percent: snapshot.disk_usage_percent ?? null,
    uptime_secs: snapshot.uptime_secs ?? null,
  }
}

/** 快照就地补丁：只换 report 与新鲜度字段，不动 online/session（在线翻转由 presence 事件负责）。 */
function patchFromSnapshot(
  machine: MachineSummary,
  snapshot: MachineSnapshotPoint,
): MachineSummary {
  return {
    ...machine,
    report: snapshotReport(snapshot),
    report_stale: false,
    report_age_secs: 0,
    agent_online: true,
  }
}

// 同一帧内的多台机器快照合并成一次 items 重赋值，避免突发流量下反复触发渲染。
const pendingSnapshots = new Map<string, MachineSnapshotPoint>()
let snapshotFrame: number | null = null

function flushSnapshots() {
  snapshotFrame = null
  if (!pendingSnapshots.size) return
  const batch = new Map(pendingSnapshots)
  pendingSnapshots.clear()
  items.value = items.value.map((machine) => {
    const snapshot = batch.get(machine.serial)
    return snapshot ? patchFromSnapshot(machine, snapshot) : machine
  })
}

function handleSnapshot(snapshot: MachineSnapshotPoint) {
  if (items.value.some((machine) => machine.serial === snapshot.serial)) {
    pendingSnapshots.set(snapshot.serial, snapshot)
    if (snapshotFrame === null) snapshotFrame = requestAnimationFrame(flushSnapshots)
  }

  // 新注册设备不在当前页：做一次防抖全量对齐，由 items watcher 负责「新机器」toast。
  if (!knownSerials || knownSerials.has(snapshot.serial)) return
  if (reconciledSnapshotSerials.has(snapshot.serial)) return
  reconciledSnapshotSerials.add(snapshot.serial)
  refreshFromStream()
}

function onPresenceEvent(event: PresenceEvent) {
  // 只处理当前页可见机器；其余事件忽略，绝不触发列表重载。
  if (!items.value.some((machine) => machine.serial === event.serial)) return

  const receivedAt = Date.now()
  items.value = items.value.map((machine) =>
    machine.serial === event.serial
      ? {
          ...machine,
          online: event.current_state,
          session_online: event.current_state,
          last_seen_at: event.occurred_at,
        }
      : machine,
  )
  recentChanges.value = {
    ...recentChanges.value,
    [event.serial]: { state: event.current_state, at: receivedAt },
  }

  if (receivedAt - mountedAt < PRESENCE_TOAST_GRACE_MS) return
  const lastToastAt = presenceToastAt.get(event.serial)
  if (lastToastAt !== undefined && receivedAt - lastToastAt < PRESENCE_TOAST_DEDUPE_MS) return
  presenceToastAt.set(event.serial, receivedAt)

  const description = `${event.device_slot} · ${shortId(event.serial, 10)}`
  if (event.current_state === 'online') toast.info('机器已上线', description)
  else toast.error('机器已离线', description)
}

const { status: snapshotStatus, lastEventAt } = useMachineStream({ onSnapshot: handleSnapshot })
const { status: presenceStatus } = usePresenceStream({ onEvent: onPresenceEvent })

const streamStatus = computed<MachineStreamStatus>(() => {
  if (snapshotStatus.value === 'live' || presenceStatus.value === 'live') return 'live'
  if (snapshotStatus.value === 'connecting' || presenceStatus.value === 'connecting')
    return 'connecting'
  return 'offline'
})

const nowTick = ref(Date.now())
useInterval(() => {
  nowTick.value = Date.now()
  pruneRecentChanges(nowTick.value)
}, 1_000)

const streamAgeLabel = computed(() => {
  if (!lastEventAt.value) return ''
  const seconds = Math.max(0, Math.round((nowTick.value - Date.parse(lastEventAt.value)) / 1000))
  return `${seconds}s 前推送`
})

const streamMeta = computed<{ label: string; tone: 'emerald' | 'amber' | 'rose' }>(() => {
  if (streamStatus.value === 'live') return { label: '实时流已连接', tone: 'emerald' }
  if (streamStatus.value === 'connecting') return { label: '实时流连接中', tone: 'amber' }
  return { label: '实时流重连中', tone: 'rose' }
})

onMounted(async () => {
  const queryArea = typeof route.query.area_id === 'string' ? route.query.area_id : ''
  const queryRoom = typeof route.query.room_id === 'string' ? route.query.room_id : ''
  if (queryArea) areaId.value = queryArea
  if (queryRoom) roomId.value = queryRoom
  await Promise.all([load(), loadStats(), loadOptions()])
  if (!unmounted) startRefreshTimer()
})

onBeforeUnmount(() => {
  unmounted = true
  stopRefreshTimer()
  if (snapshotFrame !== null) {
    cancelAnimationFrame(snapshotFrame)
    snapshotFrame = null
  }
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="机器总览" badge="room-agent">
      <template #description>
        房间机器的主机清单、健康评分、诊断与历史快照；数据由 room-agent 周期上报。
      </template>
      <template #actions>
        <StateBadge
          :label="streamMeta.label"
          :tone="streamMeta.tone"
          :pulse="streamStatus === 'live'"
        />
        <span v-if="streamAgeLabel" class="text-[10.5px] tabular-nums text-faint">
          {{ streamAgeLabel }}
        </span>
        <IconButton
          icon="refresh"
          label="刷新机器列表"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
      </template>
    </PageHeader>

    <section class="mt-4 grid grid-cols-4 gap-4" aria-label="机器关键指标">
      <MetricCard v-for="card in statCards" :key="card.label" v-bind="card" />
    </section>

    <div class="mt-4 flex flex-wrap items-center gap-2">
      <SearchInput
        v-model="keyword"
        placeholder="搜索主机名、serial、槽位或房间"
        class="w-[280px]"
      />
      <select v-model="areaId" class="select w-[170px]" aria-label="按区域筛选">
        <option value="">全部区域</option>
        <option v-for="area in areas" :key="area.id" :value="area.id">{{ area.name }}</option>
      </select>
      <select v-model="roomId" class="select w-[170px]" aria-label="按房间筛选">
        <option value="">全部房间</option>
        <option v-for="room in visibleRooms" :key="room.id" :value="room.id">
          {{ room.name }}
        </option>
      </select>
      <select v-model="health" class="select w-[150px]" aria-label="按健康状态筛选">
        <option value="">全部健康状态</option>
        <option v-for="option in MACHINE_HEALTH_FILTERS" :key="option.value" :value="option.value">
          {{ option.label }}
        </option>
      </select>
      <select
        v-model="machineStateFilter"
        class="select w-[150px]"
        aria-label="按观测状态筛选"
      >
        <option value="">全部观测状态</option>
        <option value="online">在线</option>
        <option value="stale">遥测过期</option>
        <option value="offline">离线</option>
        <option value="missing">未上报</option>
      </select>
      <button
        v-if="keyword || areaId || roomId || health || machineStateFilter"
        type="button"
        class="text-[11.5px] text-accent transition-colors hover:text-accent-strong"
        @click="resetFilters"
      >
        重置筛选
      </button>
      <div class="ml-auto flex items-center gap-2">
        <label for="machines-refresh-mode" class="text-[11px] text-faint">刷新</label>
        <select
          id="machines-refresh-mode"
          v-model="refreshMode"
          class="select w-[104px]"
          aria-label="机器列表刷新频率"
        >
          <option v-for="option in REFRESH_OPTIONS" :key="option.value" :value="option.value">
            {{ option.label }}
          </option>
        </select>
        <span class="text-[11px] text-faint">共 {{ total }} 台机器</span>
      </div>
    </div>

    <div
      v-if="error"
      role="alert"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <button type="button" class="font-medium underline" @click="load()">重试</button>
    </div>

    <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
      <table class="w-full border-collapse text-left">
        <caption class="sr-only">
          机器总览
        </caption>
        <thead>
          <tr class="border-b border-line bg-canvas/50 text-[10.5px] text-faint">
            <th scope="col" class="table-th pl-5">主机</th>
            <th scope="col" class="table-th">设备</th>
            <th scope="col" class="table-th">位置</th>
            <th scope="col" class="table-th">机器 / 会话</th>
            <th scope="col" class="table-th">健康</th>
            <th scope="col" class="table-th w-[190px]">资源</th>
            <th scope="col" class="table-th">运行时长</th>
            <th scope="col" class="table-th pr-5 text-right">最近上报</th>
          </tr>
        </thead>
        <tbody>
          <template v-if="loading && !items.length">
            <tr v-for="index in 6" :key="index" class="border-b border-line/70">
              <td colspan="8" class="px-5 py-3">
                <div class="h-9 animate-pulse rounded bg-canvas" />
              </td>
            </tr>
          </template>

          <template v-else>
            <tr
              v-for="machine in items"
              :key="machine.serial"
              class="cursor-pointer border-b border-line/70 transition-colors duration-700 last:border-b-0 hover:bg-canvas/50"
              :class="rowToneClasses(machine.serial)"
              @click="openMachine(machine.serial)"
            >
              <td class="py-3 pl-5 pr-3">
                <p class="text-[12.5px] font-medium text-ink">
                  {{ machine.report?.hostname || '未上报主机' }}
                </p>
                <p class="mt-0.5 truncate text-[10.5px] text-faint">
                  {{ machine.report?.os_name || '—' }}
                  <span v-if="machine.report?.kernel_version"
                    >· {{ machine.report.kernel_version }}</span
                  >
                </p>
              </td>
              <td class="px-3 py-3">
                <p class="font-mono text-[11px] text-ink-soft">{{ machine.device_slot }}</p>
                <p class="mt-0.5 font-mono text-[10px] text-faint">
                  {{ shortId(machine.serial, 10) }}
                </p>
              </td>
              <td class="px-3 py-3 text-[11px] text-muted">
                <p>{{ machine.area_name ?? '未绑定区域' }}</p>
                <p class="mt-0.5 text-[10.5px] text-faint">
                  {{ machine.room_name ?? machine.room_code ?? '未绑定房间' }}
                </p>
              </td>
              <td class="px-3 py-3">
                <div class="flex flex-col items-start gap-1">
                  <div class="flex flex-wrap items-center gap-1.5">
                    <StateBadge v-bind="machineStateMeta(machine)" />
                    <span
                      v-if="recentChangeFor(machine.serial)"
                      class="rounded px-1 py-0.5 text-[9.5px] font-medium"
                      :class="recentChangeClasses(machine.serial)"
                    >
                      {{ recentChangeLabel(machine.serial) }}
                    </span>
                  </div>
                  <span class="text-[10px] text-faint">
                    device 会话 · {{ machine.session_online ?? '无' }}
                  </span>
                </div>
              </td>
              <td class="px-3 py-3">
                <div class="flex items-center gap-2">
                  <StateBadge
                    :label="
                      machine.report
                        ? machineHealthMeta(machine.report.health_status).label
                        : '未上报'
                    "
                    :tone="
                      machine.report
                        ? machineHealthMeta(machine.report.health_status).tone
                        : 'slate'
                    "
                  />
                  <span
                    v-if="
                      machine.report?.health_score !== null &&
                      machine.report?.health_score !== undefined
                    "
                    class="font-mono text-[10.5px] text-muted"
                  >
                    {{ machine.report.health_score }}
                  </span>
                </div>
              </td>
              <td class="px-3 py-3">
                <div v-if="machine.report" class="flex flex-col gap-1">
                  <UsageBar :value="machine.report.cpu_usage_percent" label="CPU" />
                  <UsageBar :value="machine.report.memory_usage_percent" label="MEM" />
                  <UsageBar :value="machine.report.disk_usage_percent" label="DISK" />
                </div>
                <span v-else class="text-[10.5px] text-faint">—</span>
              </td>
              <td class="px-3 py-3 font-mono text-[11px] text-muted">
                {{ machine.report ? formatSeconds(machine.report.uptime_secs) : '—' }}
              </td>
              <td class="py-3 pl-3 pr-5 text-right">
                <template v-if="machine.report">
                  <p class="text-[11px] tabular-nums text-muted">
                    {{ formatRelative(machine.report.collected_at) }}
                  </p>
                  <p v-if="machine.report_stale" class="mt-0.5 text-[10px] text-amber-600">
                    快照已过期
                    <template v-if="machine.report_age_secs !== null">
                      · {{ formatSeconds(machine.report_age_secs) }} 前
                    </template>
                  </p>
                </template>
                <span v-else class="text-[10.5px] text-faint">—</span>
              </td>
            </tr>

            <tr v-if="!items.length">
              <td colspan="8">
                <EmptyState
                  title="没有匹配的机器"
                  description="调整筛选条件，或先让房间机器运行 room-agent 上报第一份快照。"
                  icon="cpu"
                >
                  <template #action>
                    <button
                      v-if="keyword || areaId || roomId || health || machineStateFilter"
                      type="button"
                      class="text-[11.5px] text-accent hover:underline"
                      @click="resetFilters"
                    >
                      重置筛选
                    </button>
                  </template>
                </EmptyState>
              </td>
            </tr>
          </template>
        </tbody>
      </table>

      <PaginationBar
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="台机器"
        @change="changePage"
      />
    </section>
  </div>
</template>
