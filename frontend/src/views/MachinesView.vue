<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api } from '@/api'
import type { Area, MachineSummary, Room } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useInterval } from '@/composables/useInterval'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import { MACHINE_HEALTH_FILTERS, machineHealthMeta } from '@/features/machines/machineMeta'
import { useMachineStream } from '@/features/machines/useMachineStream'
import UsageBar from '@/features/machines/UsageBar.vue'
import { errorMessage } from '@/utils/errors'
import { formatRelative, formatSeconds, shortId } from '@/utils/format'

const router = useRouter()
const route = useRoute()

const PAGE_SIZE = 20

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

const stats = ref({ total: 0, online: 0, alerts: 0, missing: 0 })
const statsLoading = ref(false)

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

const visibleRooms = computed(() =>
  areaId.value ? rooms.value.filter((room) => room.area_id === areaId.value) : rooms.value,
)

function agentOnline(machine: MachineSummary): boolean {
  return machine.report !== null && !machine.report_stale
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
    note: '最近 5 分钟内有 room-agent 上报',
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
    const data = await api.listMachines({ page_size: 100 })
    const all = data.items
    stats.value = {
      total: data.total,
      online: all.filter((item) => item.report !== null && !item.report_stale).length,
      alerts: all.filter(
        (item) =>
          item.report?.health_status === 'warning' || item.report?.health_status === 'critical',
      ).length,
      missing: all.filter((item) => item.report === null).length,
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

function resetFilters() {
  keyword.value = ''
  areaId.value = ''
  roomId.value = ''
  health.value = ''
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
}, 400)

const { status: streamStatus, lastEventAt } = useMachineStream({
  onSnapshot: () => refreshFromStream(),
})

const nowTick = ref(Date.now())
useInterval(() => {
  nowTick.value = Date.now()
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
      <button
        v-if="keyword || areaId || roomId || health"
        type="button"
        class="text-[11.5px] text-accent transition-colors hover:text-accent-strong"
        @click="resetFilters"
      >
        重置筛选
      </button>
      <span class="ml-auto text-[11px] text-faint">共 {{ total }} 台机器</span>
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
              class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
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
                  <StateBadge
                    :label="
                      agentOnline(machine) ? 'Agent 在线' : machine.report ? 'Agent 过期' : '未上报'
                    "
                    :tone="agentOnline(machine) ? 'emerald' : machine.report ? 'amber' : 'slate'"
                  />
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
                      v-if="keyword || areaId || roomId || health"
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
