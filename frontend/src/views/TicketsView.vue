<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { Room, Ticket, TicketAssignee, TicketStatus, TicketSummary } from '@/api/schema'
import type { TicketListParams } from '@/api/types'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

import TicketCreateModal from '@/features/tickets/TicketCreateModal.vue'
import TicketFilters from '@/features/tickets/TicketFilters.vue'
import TicketTable from '@/features/tickets/TicketTable.vue'
import { TICKET_STATUS_ORDER, ticketStatusMeta } from '@/features/tickets/ticketMeta'
import { buildStatsRange } from '@/features/tickets/ticketStats'
import { ticketStreamStatusMeta, useTicketStream } from '@/features/tickets/useTicketStream'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()
const route = useRoute()
const router = useRouter()

function queryString(key: string): string {
  const value = route.query[key]
  return typeof value === 'string' ? value : ''
}

const status = ref(queryString('status'))
const priority = ref(queryString('priority'))
const category = ref(queryString('category'))
const assignee = ref(queryString('assignee'))
const range = ref(queryString('range'))
const keyword = ref(queryString('keyword'))
const mine = ref(route.query.mine === '1')
const page = ref(Math.max(1, Number(queryString('page') || '1') || 1))

const tickets = ref<Ticket[]>([])
const total = ref(0)
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)

const summary = ref<TicketSummary | null>(null)
const summaryLoading = ref(false)
const assignees = ref<TicketAssignee[]>([])
const rooms = ref<Room[]>([])
const createOpen = ref(false)
const exporting = ref(false)
const createInitial = ref({
  roomId: '',
  serial: '',
  deviceSlot: '',
  title: '',
})

const canCreate = computed(() => auth.can(PERMISSIONS.ticketCreate))
const canExport = computed(() => auth.canAny(PERMISSIONS.ticketExport, PERMISSIONS.ticketRead))

/** 工单事件实时流：收到事件后刷新当前列表与 KPI（400ms 防抖由 hook 负责）。 */
const { status: streamStatus } = useTicketStream({
  onEvent: () => {
    void load()
    void loadSummary()
  },
})
const streamMeta = computed(() => ticketStreamStatusMeta(streamStatus.value))

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

const hasFilter = computed(() =>
  Boolean(
    status.value ||
    priority.value ||
    category.value ||
    assignee.value ||
    range.value ||
    keyword.value.trim() ||
    mine.value,
  ),
)

/** 后端 by_status 是 `BTreeMap<String, i64>`，生成的 TS 是 Record<string, unknown>，这里统一收敛成数字。 */
function statusCount(key: TicketStatus): number {
  const value = summary.value?.by_status?.[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

const statusTabs = computed(() => {
  let all = 0
  for (const key of TICKET_STATUS_ORDER) all += statusCount(key)
  return [
    { value: '', label: '全部', count: all },
    ...TICKET_STATUS_ORDER.map((key) => ({
      value: key,
      label: ticketStatusMeta(key).label,
      count: statusCount(key),
    })),
  ]
})

const roomNames = computed(() => {
  const map: Record<string, string> = {}
  for (const room of rooms.value) map[room.id] = room.name
  return map
})

const kpis = computed(() => {
  return [
    {
      key: 'open',
      label: '待处理',
      value: summary.value ? statusCount('open') : '—',
      note: `其中未分配 ${summary.value?.unassigned ?? 0} 张`,
      icon: 'ticket',
      tone: 'amber',
    },
    {
      key: 'in_progress',
      label: '处理中',
      value: summary.value ? statusCount('in_progress') : '—',
      note: `我的待办 ${summary.value?.mine_active ?? 0} 张`,
      icon: 'clock',
      tone: 'accent',
    },
    {
      key: 'resolved',
      label: '已解决',
      value: summary.value ? statusCount('resolved') : '—',
      note: '等待关闭确认',
      icon: 'check-circle',
      tone: 'emerald',
    },
    {
      key: 'overdue',
      label: '超时',
      value: summary.value ? (summary.value.overdue ?? 0) : '—',
      note: 'SLA 已过期且未关闭',
      icon: 'alert',
      tone: 'rose',
    },
  ]
})

const kpiToneClass: Record<string, string> = {
  amber: 'text-amber-600',
  accent: 'text-accent',
  emerald: 'text-emerald-600',
  rose: 'text-rose-600',
}

function syncQuery() {
  const query: Record<string, string> = {}
  if (status.value) query.status = status.value
  if (priority.value) query.priority = priority.value
  if (category.value) query.category = category.value
  if (assignee.value) query.assignee = assignee.value
  if (range.value) query.range = range.value
  if (mine.value) query.mine = '1'
  if (keyword.value.trim()) query.keyword = keyword.value.trim()
  if (page.value > 1) query.page = String(page.value)
  void router.replace({ query })
}

function buildParams(): TicketListParams {
  const params: TicketListParams = {
    page: page.value,
    page_size: PAGE_SIZE,
    status: (status.value || undefined) as TicketStatus | undefined,
    priority: (priority.value || undefined) as TicketListParams['priority'],
    category: (category.value || undefined) as TicketListParams['category'],
    keyword: keyword.value.trim() || undefined,
    mine: mine.value || undefined,
  }
  if (assignee.value === 'unassigned') {
    params.unassigned = true
  } else if (assignee.value) {
    params.assignee_id = assignee.value
  }
  if (range.value) {
    params.from = buildStatsRange(Number(range.value)).from
  }
  return params
}

async function load() {
  loading.value = true
  error.value = ''
  forbidden.value = false
  try {
    const data = await api.listTickets(buildParams())
    tickets.value = data.items ?? []
    total.value = data.total ?? 0
    const pages = Math.max(1, Math.ceil(total.value / PAGE_SIZE))
    if (page.value > pages) {
      page.value = pages
      syncQuery()
      void load()
      return
    }
  } catch (caught) {
    tickets.value = []
    total.value = 0
    if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载工单列表失败')
    }
  } finally {
    loading.value = false
  }
}

async function loadSummary() {
  summaryLoading.value = true
  try {
    summary.value = await api.ticketSummary()
  } catch {
    summary.value = null
  } finally {
    summaryLoading.value = false
  }
}

async function loadOptions() {
  try {
    rooms.value = (await api.listRooms({ page: 1, page_size: 100 })).items ?? []
  } catch {
    rooms.value = []
  }
  try {
    assignees.value = await api.ticketAssignees()
  } catch {
    assignees.value = []
  }
}

const { run: debouncedSearch } = useDebounceFn(() => reloadFromFirstPage(), 320)

function reloadFromFirstPage() {
  page.value = 1
  syncQuery()
  void load()
}

function changePage(next: number) {
  page.value = next
  syncQuery()
  void load()
}

function changeStatus(value: string | number) {
  status.value = String(value)
}

function resetFilters() {
  status.value = ''
  priority.value = ''
  category.value = ''
  assignee.value = ''
  range.value = ''
  mine.value = false
  // 以上 ref 的变化会被下面的 watch 合并成一次 reloadFromFirstPage，这里不再直接触发请求。
}

function refresh() {
  void Promise.all([load(), loadSummary(), loadOptions()])
}

function exportFileName(): string {
  const now = new Date()
  const pad = (value: number) => String(value).padStart(2, '0')
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}`
  return `tickets-${date}-${time}.csv`
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  URL.revokeObjectURL(url)
}

/** 导出与当前列表筛选一致的数据（CSV 不分页，后端上限 5000 行）。 */
async function exportCsv() {
  if (exporting.value) return
  exporting.value = true
  try {
    const params: TicketListParams = { ...buildParams() }
    delete params.page
    delete params.page_size
    const blob = await api.exportTickets(params)
    const filename = exportFileName()
    downloadBlob(blob, filename)
    toast.success('工单已导出', filename)
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '当前账号没有 ticket:export 权限。')
    } else {
      toast.error('导出失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    exporting.value = false
  }
}

function openCreate() {
  createInitial.value = { roomId: '', serial: '', deviceSlot: '', title: '' }
  createOpen.value = true
}

/**
 * 房间 / 设备 / 机器详情页「报修 / 建单」入口通过 query 预填：
 * `?new=1&room_id=&serial=&device_slot=&title=`；打开弹窗后清掉这些参数，避免刷新重复弹窗。
 */
function applyCreatePrefill() {
  if (route.query.new !== '1' || !canCreate.value) return
  createInitial.value = {
    roomId: queryString('room_id'),
    serial: queryString('serial'),
    deviceSlot: queryString('device_slot'),
    title: queryString('title'),
  }
  createOpen.value = true
  const query = { ...route.query }
  delete query.new
  delete query.room_id
  delete query.serial
  delete query.device_slot
  delete query.title
  void router.replace({ query })
}

function onCreated(id: string) {
  createOpen.value = false
  createInitial.value = { roomId: '', serial: '', deviceSlot: '', title: '' }
  void router.push({ name: 'ticket-detail', params: { id } })
}

function openDetail(ticket: Ticket) {
  void router.push({ name: 'ticket-detail', params: { id: ticket.id } })
}

watch(keyword, () => debouncedSearch())
watch([status, priority, category, assignee, mine, range], () => reloadFromFirstPage())

onMounted(() => {
  applyCreatePrefill()
  void load()
  void loadSummary()
  void loadOptions()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="工单"
      description="统一承接机器健康、设备离线、推送 / 分发失败与人工上报，形成可追踪的处理闭环。"
    >
      <template #actions>
        <SearchInput v-model="keyword" placeholder="搜索编号 / 标题 / 房间 / 设备" />
        <AppButton v-if="canExport" icon="download" :loading="exporting" @click="exportCsv">
          导出 CSV
        </AppButton>
        <IconButton
          icon="refresh"
          label="刷新工单列表"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
        <AppButton icon="chart" @click="router.push({ name: 'ticket-insights' })"
          >工单分析</AppButton
        >
        <AppButton v-if="canCreate" icon="plus" variant="primary" @click="openCreate"
          >新建工单</AppButton
        >
      </template>
    </PageHeader>

    <section class="mt-4 grid grid-cols-4 gap-3" aria-label="工单关键指标">
      <article
        v-for="kpi in kpis"
        :key="kpi.key"
        class="rounded-xl border border-line bg-surface px-5 py-4"
      >
        <div class="flex items-start justify-between">
          <p class="text-[11.5px] font-medium text-muted">{{ kpi.label }}</p>
          <span
            class="flex h-6 w-6 items-center justify-center rounded-md border border-line bg-canvas/70 text-faint"
          >
            <AppIcon :name="kpi.icon" :size="13" />
          </span>
        </div>
        <p class="mt-2.5 text-[23px] font-semibold leading-none tracking-[-0.02em] tabular-nums">
          <span
            v-if="summaryLoading && !summary"
            class="inline-block h-[23px] w-12 animate-pulse rounded bg-canvas align-middle"
          />
          <span v-else :class="kpiToneClass[kpi.tone]">{{ kpi.value }}</span>
        </p>
        <p class="mt-4 truncate text-[11px] text-faint">{{ kpi.note }}</p>
      </article>
    </section>

    <div class="mt-4 flex items-center justify-between gap-3">
      <FilterTabs :model-value="status" :tabs="statusTabs" @change="changeStatus" />
      <div class="flex items-center gap-3">
        <StateBadge
          :label="streamMeta.label"
          :tone="streamMeta.tone"
          :pulse="streamStatus === 'live'"
        />
        <span class="text-[11px] text-faint">按更新时间倒序 · 共 {{ total }} 张工单</span>
      </div>
    </div>

    <div class="mt-3">
      <TicketFilters
        v-model:priority="priority"
        v-model:category="category"
        v-model:assignee="assignee"
        v-model:mine="mine"
        v-model:range="range"
        :assignees="assignees"
        @reset="resetFilters"
      />
    </div>

    <div
      v-if="error && tickets.length"
      role="alert"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
      <TicketTable
        v-if="tickets.length"
        :tickets="tickets"
        :loading="loading"
        :room-names="roomNames"
        @open="openDetail"
      />

      <div v-else-if="loading" class="space-y-3 px-5 py-6">
        <div v-for="row in 6" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
      </div>

      <EmptyState
        v-else-if="forbidden"
        title="缺少工单权限"
        description="当前账号没有 ticket:read 权限，无法查看工单列表。请联系管理员授予工单读取权限。"
        icon="lock"
      />

      <EmptyState v-else-if="error" title="加载工单失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <EmptyState
        v-else
        :title="hasFilter ? '没有匹配的工单' : '还没有工单'"
        :description="
          hasFilter
            ? '当前筛选条件下没有结果，试试调整状态、优先级、处理人、时间范围或清空搜索关键词。'
            : '机器健康、设备离线、课表推送失败会自动建单；也可以手工新建一张工单。'
        "
        icon="ticket"
      >
        <template v-if="hasFilter" #action>
          <AppButton size="sm" @click="resetFilters">清除筛选</AppButton>
        </template>
        <template v-else-if="canCreate" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建工单</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && tickets.length"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="张工单"
        @change="changePage"
      />
    </section>

    <TicketCreateModal
      :open="createOpen"
      :rooms="rooms"
      :initial-room-id="createInitial.roomId"
      :initial-serial="createInitial.serial"
      :initial-device-slot="createInitial.deviceSlot"
      :initial-title="createInitial.title"
      @close="createOpen = false"
      @created="onCreated"
    />
  </div>
</template>
