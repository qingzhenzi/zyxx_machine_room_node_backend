<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { api, isNotFound } from '@/api'
import type { MachineDetail } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useInterval } from '@/composables/useInterval'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import MachineHealthIssues from '@/features/machines/MachineHealthIssues.vue'
import MachineHistoryTable from '@/features/machines/MachineHistoryTable.vue'
import MachineOverviewSections from '@/features/machines/MachineOverviewSections.vue'
import UsageBar from '@/features/machines/UsageBar.vue'
import { healthScoreClass, machineHealthMeta } from '@/features/machines/machineMeta'
import { parseMachinePayload } from '@/features/machines/payload'
import { useMachineStream } from '@/features/machines/useMachineStream'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative, formatSeconds } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()
const route = useRoute()
const router = useRouter()

const canCreateTicket = computed(() => auth.can(PERMISSIONS.ticketCreate))
const canWake = computed(() => auth.can(PERMISSIONS.deviceWake))

/** P1 Wave 2 报修入口：带上机器 serial / 槽位 / 房间。 */
function reportIssue() {
  const current = detail.value
  const serialValue = detail.value?.serial || serial.value
  if (!serialValue) return
  void router.push({
    name: 'tickets',
    query: {
      new: '1',
      serial: serialValue,
      ...(current?.device_slot ? { device_slot: current.device_slot } : {}),
      ...(current?.room_id ? { room_id: current.room_id } : {}),
      title: `机器报修：${report.value?.hostname || current?.device_slot || serialValue}`,
    },
  })
}

const detail = ref<MachineDetail | null>(null)
const loading = ref(false)
const error = ref('')
const notFound = ref(false)
const autoRefresh = ref(false)
const wakeOpen = ref(false)
const wakeLoading = ref(false)

const serial = computed(() => String(route.params.serial ?? ''))
const report = computed(() => detail.value?.report ?? null)
const health = computed(() => machineHealthMeta(report.value?.health_status))
const parsed = computed(() =>
  detail.value?.latest ? parseMachinePayload(detail.value.latest.payload) : null,
)

/** WOL 目标 MAC：来自最新快照 `sections.agent.host.macs`（旧 agent 未上报 → 空）。 */
const wakeMacs = computed(() => parsed.value?.agent?.macs ?? [])

/** 按钮禁用原因（null = 可唤醒）。 */
const wakeDisabledReason = computed(() => {
  if (!detail.value?.latest) return '该机器尚未上报快照，暂无 MAC'
  if (!wakeMacs.value.length) return '最新快照未上报网卡 MAC（sections.agent.host.macs）'
  return ''
})

const wakeDetail = computed(() =>
  [
    `目标 MAC：${wakeMacs.value.join('、')}`,
    'UDP 广播 255.255.255.255:9，每个 MAC 发 3 次 magic packet。',
    '需与目标机在同一二层网络，且目标机 BIOS/网卡已开启 Wake-on-LAN。',
  ].join('\n'),
)

/** 机器是否「Agent 在线」：只看 room-agent 快照新鲜度，与 device-pki 的 mTLS 会话状态相互独立。 */
const agentOnline = computed(() => report.value !== null && detail.value?.report_stale !== true)

const statCards = computed<MetricCardProps[]>(() => {
  const currentReport = report.value
  return [
    {
      label: '健康分',
      value:
        currentReport?.health_score === null || currentReport?.health_score === undefined
          ? '—'
          : String(currentReport.health_score),
      note: health.value.label,
      badge: report.value ? health.value.label : '未上报',
      tone: health.value.tone,
      icon: 'shield',
      loading: loading.value,
    },
    {
      label: 'CPU 使用率',
      value:
        currentReport?.cpu_usage_percent === null || currentReport?.cpu_usage_percent === undefined
          ? '—'
          : `${currentReport.cpu_usage_percent.toFixed(1)}%`,
      note: '最新快照',
      tone: 'accent',
      icon: 'cpu',
      loading: loading.value,
    },
    {
      label: '内存使用率',
      value:
        currentReport?.memory_usage_percent === null ||
        currentReport?.memory_usage_percent === undefined
          ? '—'
          : `${currentReport.memory_usage_percent.toFixed(1)}%`,
      note: '最新快照',
      tone: 'sky',
      icon: 'database',
      loading: loading.value,
    },
    {
      label: '磁盘使用率',
      value:
        currentReport?.disk_usage_percent === null ||
        currentReport?.disk_usage_percent === undefined
          ? '—'
          : `${currentReport.disk_usage_percent.toFixed(1)}%`,
      note: '根分区 / 最满挂载点',
      badge: (currentReport?.disk_usage_percent ?? 0) >= 90 ? '快满' : undefined,
      tone: (currentReport?.disk_usage_percent ?? 0) >= 90 ? 'rose' : 'violet',
      icon: 'database',
      loading: loading.value,
    },
    {
      label: '运行时长',
      value: formatSeconds(currentReport?.uptime_secs),
      note: '自上次启动',
      tone: 'slate',
      icon: 'clock',
      loading: loading.value,
    },
    {
      label: '最近上报',
      value: currentReport ? formatRelative(currentReport.collected_at) : '—',
      note: currentReport ? formatFullDateTime(currentReport.collected_at) : '尚未收到快照',
      badge: agentOnline.value ? '在线' : report.value ? '已过期' : undefined,
      tone: agentOnline.value ? 'emerald' : report.value ? 'amber' : 'slate',
      icon: 'refresh',
      loading: loading.value,
    },
  ]
})

const identityItems = computed(() => {
  const device = detail.value
  if (!device) return []
  return [
    { label: '设备序列号', value: device.serial, mono: true },
    { label: '设备槽位', value: device.device_slot, mono: true },
    { label: '设备注册状态', value: device.status },
    {
      label: '机器状态（Agent）',
      value: agentOnline.value
        ? `在线 · ${report.value ? formatRelative(report.value.collected_at) : '刚刚'}上报`
        : report.value
          ? '快照已过期'
          : '未上报',
    },
    { label: '设备在线状态（综合）', value: device.online ?? '无会话' },
    { label: 'device-pki 会话', value: device.session_online ?? '无会话' },
    {
      label: '会话最后心跳',
      value: device.last_seen_at ? formatFullDateTime(device.last_seen_at) : '—',
    },
    { label: '所属区域', value: device.area_name ?? '未绑定', mono: false },
    { label: '所属房间', value: device.room_name ?? '未绑定', mono: false },
    {
      label: 'Agent 版本',
      value: report.value ? `v${report.value.agent_version}` : '未上报',
      mono: true,
    },
    { label: '网卡 MAC', value: wakeMacs.value.join('、') || '未上报', mono: true },
  ]
})

const { run: refreshFromStream } = useDebounceFn(() => {
  void load(true)
}, 400)

const { status: streamStatus, lastEventAt } = useMachineStream({
  serial,
  onSnapshot: (snapshot) => {
    if (snapshot.serial === serial.value) refreshFromStream()
  },
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

async function wakeMachine() {
  if (!serial.value) return
  wakeLoading.value = true
  try {
    const reply = await api.wakeMachine(serial.value)
    wakeOpen.value = false
    if (reply.ok) {
      const suffix = reply.warnings.length ? `；${reply.warnings.length} 条警告` : ''
      toast.success('已发送网络唤醒', `目标 MAC：${reply.macs.join('、')}${suffix}`)
    } else {
      toast.error(
        '唤醒包发送失败',
        reply.warnings.join('；') || 'UDP 广播发送失败，请检查本机网络/广播权限',
      )
    }
  } catch (caught) {
    toast.error('网络唤醒失败', errorMessage(caught, '请稍后重试'))
  } finally {
    wakeLoading.value = false
  }
}

async function load(silent = false) {
  if (!serial.value) return
  if (!silent) loading.value = true
  error.value = ''
  notFound.value = false
  try {
    detail.value = await api.getMachine(serial.value, { history_limit: 20 })
  } catch (caught) {
    if (isNotFound(caught)) {
      notFound.value = true
    } else {
      error.value = errorMessage(caught, '机器详情加载失败')
    }
  } finally {
    loading.value = false
  }
}

function refresh() {
  void load()
}

useInterval(() => {
  if (autoRefresh.value) void load(true)
}, 30_000)

watch(
  () => route.params.serial,
  () => {
    detail.value = null
    void load()
  },
)

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <div class="mb-3">
      <RouterLink
        :to="{ name: 'machines' }"
        class="inline-flex items-center gap-1.5 text-[11.5px] text-muted transition-colors hover:text-ink"
      >
        <span aria-hidden="true">←</span>
        返回机器总览
      </RouterLink>
    </div>

    <div v-if="notFound" class="rounded-xl border border-line bg-surface">
      <EmptyState
        title="机器不存在"
        description="serial 对应的设备未注册，或链接已失效。"
        icon="cpu"
      >
        <template #action>
          <AppButton size="sm" @click="router.push({ name: 'machines' })">返回机器总览</AppButton>
        </template>
      </EmptyState>
    </div>

    <template v-else>
      <PageHeader
        :title="report?.hostname || detail?.device_slot || '机器详情'"
        :badge="report ? health.label : '未上报'"
      >
        <template #description>
          <span class="font-mono">{{ serial }}</span>
          <template v-if="detail">
            · {{ detail.device_slot }}
            <template v-if="detail.area_name"> · {{ detail.area_name }}</template>
            <template v-if="detail.room_name"> / {{ detail.room_name }}</template>
          </template>
        </template>
        <template #actions>
          <AppButton v-if="canCreateTicket" icon="ticket" @click="reportIssue">
            报修 / 建单
          </AppButton>
          <AppButton
            v-if="canWake"
            icon="zap"
            :disabled="Boolean(wakeDisabledReason)"
            :title="wakeDisabledReason || '发送 Wake-on-LAN magic packet'"
            @click="wakeOpen = true"
          >
            网络唤醒
          </AppButton>
          <StateBadge
            :label="streamMeta.label"
            :tone="streamMeta.tone"
            :pulse="streamStatus === 'live'"
          />
          <span v-if="streamAgeLabel" class="text-[10.5px] tabular-nums text-faint">
            {{ streamAgeLabel }}
          </span>
          <label class="flex items-center gap-2 text-[11.5px] text-muted">
            <input
              v-model="autoRefresh"
              type="checkbox"
              class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
            />
            自动刷新
          </label>
          <IconButton
            icon="refresh"
            label="刷新机器详情"
            variant="outline"
            :loading="loading"
            @click="refresh"
          />
        </template>
      </PageHeader>

      <div
        v-if="error"
        role="alert"
        class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
      >
        <span>{{ error }}</span>
        <button type="button" class="font-medium underline" @click="refresh">重试</button>
      </div>

      <div
        v-if="report && detail?.report_stale"
        class="mt-4 flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2.5 text-[11.5px] text-amber-700"
      >
        <span class="h-1.5 w-1.5 rounded-full bg-amber-500" />
        最新快照已超过 5 分钟未更新，Agent 可能离线或采集失败。
      </div>

      <div
        v-else-if="agentOnline && detail && detail.online !== 'online'"
        class="mt-4 flex items-start gap-2 rounded-lg border border-sky-200 bg-sky-50 px-4 py-2.5 text-[11.5px] text-sky-800"
      >
        <span class="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-sky-500" />
        <span>
          Agent 正在正常上报（最近 {{ formatRelative(report?.collected_at) }}），但 device-pki 的
          mTLS 会话状态是 <span class="font-mono">{{ detail.online ?? '无会话' }}</span
          >。 两者相互独立：机器总览以 Agent 快照判断机器是否在线，device 会话只在设备走 device-pki
          心跳时更新。
        </span>
      </div>

      <section class="mt-4 grid grid-cols-6 gap-3" aria-label="机器关键指标">
        <MetricCard v-for="card in statCards" :key="card.label" v-bind="card" />
      </section>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-8 flex min-w-0 flex-col gap-4">
          <template v-if="parsed">
            <MachineHealthIssues :issues="parsed.healthIssues" />
            <MachineOverviewSections :payload="parsed" />
          </template>

          <section v-else class="rounded-xl border border-line bg-surface">
            <EmptyState
              title="尚未收到机器快照"
              description="room-agent 上报第一份快照后，这里会展示主机清单、健康结论、诊断与历史记录。"
              icon="cpu"
            />
          </section>

          <PageSection title="原始快照" description="服务端原样保存的 room-agent 上报 JSON">
            <div class="px-5 py-4">
              <JsonBlock
                v-if="detail?.latest"
                :value="detail.latest.payload"
                label="payload"
                collapsible
                max-height="520px"
              />
              <p v-else class="text-[11.5px] text-muted">暂无快照。</p>
            </div>
          </PageSection>
        </div>

        <div class="col-span-4 flex min-w-0 flex-col gap-4">
          <PageSection title="机器身份" description="设备注册与房间/区域绑定">
            <div class="px-5 py-4">
              <InfoList v-if="detail" :items="identityItems" />
              <div class="mt-3 flex flex-wrap gap-2">
                <RouterLink
                  v-if="detail?.room_id"
                  :to="{ name: 'room-detail', params: { id: detail.room_id } }"
                  class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
                >
                  房间总览
                </RouterLink>
                <RouterLink
                  v-if="detail?.area_id"
                  :to="{ name: 'area-detail', params: { id: detail.area_id } }"
                  class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
                >
                  区域总览
                </RouterLink>
                <RouterLink
                  :to="{ name: 'device-detail', params: { serial } }"
                  class="rounded-md border border-line bg-surface px-2.5 py-1.5 text-[11px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
                >
                  设备详情
                </RouterLink>
              </div>
            </div>
          </PageSection>

          <PageSection title="健康摘要" description="来自最新快照的判定">
            <div v-if="report" class="px-5 py-4">
              <div class="flex items-center gap-4">
                <div
                  class="flex h-[72px] w-[72px] shrink-0 flex-col items-center justify-center rounded-full border-4 border-line"
                >
                  <span
                    class="text-[20px] font-semibold tabular-nums"
                    :class="healthScoreClass(report.health_score)"
                  >
                    {{ report.health_score ?? '—' }}
                  </span>
                  <span class="text-[9.5px] text-faint">健康分</span>
                </div>
                <div class="min-w-0">
                  <StateBadge
                    :label="health.label"
                    :tone="health.tone"
                    :pulse="health.tone === 'rose'"
                  />
                  <p class="mt-1.5 text-[11.5px] text-muted">
                    Agent v{{ report.agent_version }} · {{ formatRelative(report.received_at) }}接收
                  </p>
                  <p class="mt-0.5 truncate text-[10.5px] text-faint">
                    {{ report.os_name ?? '—'
                    }}<span v-if="report.kernel_version"> · {{ report.kernel_version }}</span>
                  </p>
                </div>
              </div>
              <div class="mt-4 flex flex-col gap-2">
                <UsageBar :value="report.cpu_usage_percent" label="CPU" height="md" />
                <UsageBar :value="report.memory_usage_percent" label="MEM" height="md" />
                <UsageBar :value="report.disk_usage_percent" label="DISK" height="md" />
              </div>
            </div>
            <EmptyState
              v-else
              title="暂无健康数据"
              description="等待 room-agent 首次上报。"
              icon="shield"
            />
          </PageSection>
        </div>
      </div>

      <div v-if="detail" class="mt-4">
        <MachineHistoryTable :points="detail.history ?? []" />
      </div>

      <ConfirmDialog
        :open="wakeOpen"
        title="网络唤醒"
        :description="`向 ${report?.hostname || detail?.device_slot || serial} 发送 Wake-on-LAN magic packet？`"
        :detail="wakeDetail"
        confirm-label="发送唤醒包"
        :loading="wakeLoading"
        @close="wakeOpen = false"
        @confirm="wakeMachine"
      />
    </template>
  </div>
</template>
