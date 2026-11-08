<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { api, isNotFound, streamLiveMetrics } from '@/api'
import type { ImageArtifact, LiveMetricSample, MachineDetail, VmCommandOp, VmCommandReply } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
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
import MachineMetricsChart from '@/features/machines/MachineMetricsChart.vue'
import MachineOverviewSections from '@/features/machines/MachineOverviewSections.vue'
import UsageBar from '@/features/machines/UsageBar.vue'
import {
  healthScoreClass,
  machineHealthMeta,
  workloadStateMeta,
} from '@/features/machines/machineMeta'
import { parseMachinePayload, parseVmList, type VmView } from '@/features/machines/payload'
import {
  VM_OP_LABELS,
  buildCreateArgs,
  isValidVmName,
  parseDiskInfo,
  parseExportResult,
  buildMakeImageArgs,
  isVmRunningState,
  parseMakeImageResult,
  parseSnapshotList,
  validateMakeImageForm,
  validateVmCreateForm,
  type VmCreateForm,
  type VmMakeImageForm,
  type VmDiskInfoView,
  type VmExportView,
  type VmSnapshotRow,
} from '@/features/machines/vmCommand'
import { useMachineStream } from '@/features/machines/useMachineStream'
import VncConsoleModal from '@/features/machines/VncConsoleModal.vue'
import { vncRuntimeHint } from '@/features/machines/vncSession'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatBytes, formatFullDateTime, formatRelative, formatSeconds } from '@/utils/format'
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
/** 实时模式：每 2s 请求 agent 立即上报一次 fast 快照（Pterodactyl 风格）。 */
const liveMode = ref(false)
const liveSamples = ref<LiveMetricSample[]>([])
const relayStatus = ref<'connecting' | 'live' | 'offline'>('offline')
const poweroffOpen = ref(false)
const poweroffBusy = ref(false)
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

const liveLatest = computed(() => liveSamples.value[liveSamples.value.length - 1] ?? null)

function percentLabel(value: number | null | undefined): string {
  return value == null ? '—' : `${value.toFixed(1)}%`
}

const statCards = computed<MetricCardProps[]>(() => {
  const currentReport = report.value
  const live = liveLatest.value
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
      value: percentLabel(live?.cpu_usage_percent ?? currentReport?.cpu_usage_percent),
      note: live?.cpu_usage_percent != null ? '实时中转' : '最新快照',
      tone: 'accent',
      icon: 'cpu',
      loading: loading.value,
    },
    {
      label: '内存使用率',
      value: percentLabel(live?.memory_usage_percent ?? currentReport?.memory_usage_percent),
      note: live?.memory_usage_percent != null ? '实时中转' : '最新快照',
      tone: 'sky',
      icon: 'database',
      loading: loading.value,
    },
    {
      label: '磁盘使用率',
      value: percentLabel(live?.disk_usage_percent ?? currentReport?.disk_usage_percent),
      note: live?.disk_usage_percent != null ? '实时中转' : '根分区 / 最满挂载点',
      badge: (live?.disk_usage_percent ?? currentReport?.disk_usage_percent ?? 0) >= 90 ? '快满' : undefined,
      tone: (live?.disk_usage_percent ?? currentReport?.disk_usage_percent ?? 0) >= 90 ? 'rose' : 'violet',
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

// ---------------------------------------------------------------------------
// 虚拟机远程控制（vm:control）：命令经 device_downlink 下发，轮询 machine_command_results。
// 阶段 1.5：底包/差分盘/快照/磁盘链/导出（args 契约见 docs/vm-base-image-and-diff-disk.md §3）。
// ---------------------------------------------------------------------------

const VM_CONFIRM_OPS: VmCommandOp[] = [
  'start',
  'shutdown',
  'reboot',
  'destroy',
  'delete',
  'snapshot.restore',
  'snapshot.delete',
]
const VM_REFRESH_OPS: VmCommandOp[] = ['start', 'shutdown', 'reboot', 'destroy', 'delete', 'create']
const VM_POLL_INTERVAL_MS = 1000
const VM_POLL_MAX_ATTEMPTS = 60

const canVmControl = computed(() => auth.can(PERMISSIONS.vmControl))
const canRequestTelemetry = computed(() => auth.can(PERMISSIONS.deviceManage))
const canViewLive = computed(() => auth.can(PERMISSIONS.deviceRead))
const chartPoints = computed(() => detail.value?.history ?? [])
/** `list` 命令返回的列表覆盖遥测快照（刷新按钮/操作后的真实状态）。 */
const vmOverride = ref<VmView[] | null>(null)
const vmRows = computed<VmView[]>(() => vmOverride.value ?? parsed.value?.vms ?? [])
const vmBusy = ref(false)
const vmBusyName = ref('')

// 确认框（生命周期 / 快照恢复/删除）
const vmConfirm = ref<{ op: VmCommandOp; name: string; snapshot?: string } | null>(null)
// 截图
const vmScreenshot = ref<{ name: string; src: string } | null>(null)
// noVNC 实时控制台（需要 vm:control；仅运行中的虚拟机可连接）
const vncTarget = ref<{ vm: string } | null>(null)
// 新建虚拟机
const vmCreateOpen = ref(false)
const vmCreateError = ref('')
const vmBaseImages = ref<ImageArtifact[]>([])
const vmBaseImagesLoading = ref(false)
const vmCreateForm = ref<VmCreateForm>({
  name: '',
  vcpus: 2,
  memoryMiB: 2048,
  diskGiB: 20,
  baseVolume: '',
  start: true,
  gpu: '',
})
// 快照抽屉
const vmSnapshotOpen = ref(false)
const vmSnapshotVm = ref('')
const vmSnapshotInput = ref('')
const vmSnapshots = ref<VmSnapshotRow[]>([])
// 磁盘链
const vmDiskOpen = ref(false)
const vmDiskVm = ref('')
const vmDiskInfo = ref<VmDiskInfoView | null>(null)
// 导出
const vmExportOpen = ref(false)
const vmExportVm = ref('')
const vmExportResult = ref<VmExportView | null>(null)

/** 底包候选：已发布 vm_disk 镜像的 libvirt 卷名（`<image name>-<version>`，由导入器落盘）。 */
const vmBaseVolumeOptions = computed(() =>
  vmBaseImages.value.map((image) => ({
    value: `${image.name}-${image.version}`,
    label: `${image.name}-${image.version}`,
    description: `${image.platform} · ${image.format}`,
  })),
)

const vmConfirmDetail = computed(() => {
  const pending = vmConfirm.value
  if (!pending) return ''
  return [
    `虚拟机：${pending.name}`,
    pending.snapshot ? `快照：${pending.snapshot}` : '',
    `操作：${VM_OP_LABELS[pending.op]}`,
    '命令经设备下行交给机器上的 room-agent 执行，最长等待 60s。',
    pending.op === 'destroy' ? '销毁不可撤销，请确认磁盘数据已不需要。' : '',
  ]
    .filter(Boolean)
    .join('\n')
})

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function pollVmCommand(
  requestId: string,
  maxAttempts: number = VM_POLL_MAX_ATTEMPTS,
): Promise<VmCommandReply> {
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    const reply = await api.vmCommandResult(serial.value, requestId)
    if (reply.status !== 'pending') return reply
    await sleep(VM_POLL_INTERVAL_MS)
  }
  // 后端 pending>120s 会转 timeout；前端只等 60s，超时返回本地 timeout 以便提示。
  const now = new Date().toISOString()
  return {
    request_id: requestId,
    status: 'timeout',
    data: null,
    error: null,
    created_at: now,
    updated_at: now,
  }
}

/** 只负责下发 + 轮询 + 错误 toast；成功后的 UI 状态由调用方处理。 */
async function sendVmCommand(
  op: VmCommandOp,
  args: Record<string, unknown>,
  label: string,
  pollAttempts: number = VM_POLL_MAX_ATTEMPTS,
): Promise<VmCommandReply | null> {
  if (!serial.value || vmBusy.value) return null
  vmBusy.value = true
  vmBusyName.value = label
  try {
    const accepted = await api.vmCommand(serial.value, { op, args })
    const reply = await pollVmCommand(accepted.request_id, pollAttempts)
    if (reply.status === 'ok') return reply
    if (reply.status === 'timeout') {
      toast.error(`${label}超时`, '60s 内未收到机器回报，请确认机器在线且 agent 正常')
    } else {
      toast.error(`${label}失败`, reply.error ?? '机器返回错误')
    }
    return reply
  } catch (caught) {
    toast.error(`${label}失败`, errorMessage(caught, '请稍后重试'))
    return null
  } finally {
    vmBusy.value = false
    vmBusyName.value = ''
  }
}

/** 通用命令：刷新/生命周期/截图；成功后更新对应 UI 并刷新列表。 */
async function runVmCommand(op: VmCommandOp, args: Record<string, unknown> = {}) {
  const label = VM_OP_LABELS[op]
  const reply = await sendVmCommand(op, args, label)
  if (!reply || reply.status !== 'ok') return reply

  if (op === 'list') {
    const vms = parseVmList(reply.data)
    if (vms) vmOverride.value = vms
    toast.success('已刷新虚拟机列表')
    return reply
  }
  if (op === 'screenshot') {
    const data = (reply.data ?? {}) as Record<string, unknown>
    const image = typeof data.image_base64 === 'string' ? data.image_base64 : ''
    if (!image) {
      toast.error('截图失败', '机器没有返回截图数据（检查 libvirt / 显卡驱动）')
      return reply
    }
    const format = typeof data.format === 'string' ? data.format : 'png'
    vmScreenshot.value = {
      name: typeof args.name === 'string' ? args.name : '虚拟机',
      src: `data:image/${format};base64,${image}`,
    }
    toast.success('截图完成')
    return reply
  }

  toast.success(`${label}成功`, typeof args.name === 'string' ? `虚拟机：${args.name}` : undefined)
  if (VM_REFRESH_OPS.includes(op)) void runVmCommand('list')
  return reply
}

/** 启动/关机/重启/销毁/快照恢复/删除先弹确认框；截图/刷新直接下发。 */
function requestVmOp(op: VmCommandOp, name: string) {
  if (VM_CONFIRM_OPS.includes(op)) {
    vmConfirm.value = { op, name }
    return
  }
  void runVmCommand(op, { name })
}

function confirmVmOp() {
  const pending = vmConfirm.value
  vmConfirm.value = null
  if (!pending) return
  if (pending.op === 'snapshot.restore' || pending.op === 'snapshot.delete') {
    void runSnapshotMutation(pending.op, pending.name, pending.snapshot ?? '')
    return
  }
  if (pending.op === 'delete') {
    // 控制面删除默认连带清理磁盘（与 CLI rm --with-disks 语义一致）。
    void runVmCommand('delete', { name: pending.name, remove_disks: true })
    return
  }
  void runVmCommand(pending.op, { name: pending.name })
}

function refreshVmList() {
  void runVmCommand('list')
}

function screenshotVm(name: string) {
  void runVmCommand('screenshot', { name })
}

/** 打开 noVNC 实时控制台；libvirt 只在虚拟机运行时提供 VNC 显示。 */
function openVncConsole(name: string) {
  const state = vmRows.value.find((vm) => vm.name === name)?.state
  if (!isVmRunningState(state)) {
    toast.error('虚拟机未运行', `当前状态为 ${state ?? '未上报'}，请先启动后再打开控制台`)
    return
  }
  vncTarget.value = { vm: name }
}

// ---- 新建虚拟机（底包 + 差分盘） ----

async function loadVmBaseImages() {
  vmBaseImagesLoading.value = true
  try {
    const page = await api.listImages({
      kind: 'vm_disk',
      status: 'published',
      page: 1,
      page_size: 50,
    })
    vmBaseImages.value = page.items ?? []
  } catch {
    vmBaseImages.value = []
  } finally {
    vmBaseImagesLoading.value = false
  }
}

function openVmCreate() {
  vmCreateError.value = ''
  vmCreateForm.value = {
    name: '',
    vcpus: 2,
    memoryMiB: 2048,
    diskGiB: 20,
    baseVolume: vmBaseVolumeOptions.value[0]?.value ?? '',
    start: true,
    gpu: '',
  }
  vmCreateOpen.value = true
  if (!vmBaseImages.value.length) void loadVmBaseImages()
}

async function submitVmCreate() {
  const error = validateVmCreateForm(vmCreateForm.value)
  vmCreateError.value = error ?? ''
  if (error) return
  const form = vmCreateForm.value
  const reply = await sendVmCommand('create', buildCreateArgs(form), '新建虚拟机')
  if (!reply || reply.status !== 'ok') return
  vmCreateOpen.value = false
  const data = (reply.data ?? {}) as Record<string, unknown>
  const diskPath = typeof data.disk_path === 'string' ? data.disk_path : ''
  toast.success('虚拟机创建成功', diskPath ? `${form.name} · ${diskPath}` : form.name)
  void runVmCommand('list')
}

// ---- 快照（关机状态下 qcow2 内部快照） ----

async function openVmSnapshots(name: string) {
  vmSnapshotVm.value = name
  vmSnapshotInput.value = ''
  vmSnapshots.value = []
  vmSnapshotOpen.value = true
  await refreshVmSnapshots()
}

async function refreshVmSnapshots() {
  if (!vmSnapshotVm.value) return
  const reply = await sendVmCommand('snapshot.list', { name: vmSnapshotVm.value }, '加载快照列表')
  if (reply?.status === 'ok') {
    vmSnapshots.value = parseSnapshotList(reply.data)
  }
}

function snapshotNameError(): string {
  const value = vmSnapshotInput.value.trim()
  if (!value) return '请填写快照名'
  if (!isValidVmName(value)) return '快照名只允许字母、数字、点、下划线、短横线（1-64 字符）'
  return ''
}

async function createVmSnapshot() {
  const error = snapshotNameError()
  if (error) {
    toast.error('快照名非法', error)
    return
  }
  const snapshot = vmSnapshotInput.value.trim()
  const reply = await sendVmCommand(
    'snapshot.create',
    { name: vmSnapshotVm.value, snapshot },
    '创建快照',
  )
  if (!reply || reply.status !== 'ok') return
  toast.success('快照已创建', snapshot)
  vmSnapshotInput.value = ''
  await refreshVmSnapshots()
}

function requestSnapshotRestore(tag: string) {
  vmConfirm.value = { op: 'snapshot.restore', name: vmSnapshotVm.value, snapshot: tag }
}

function requestSnapshotDelete(tag: string) {
  vmConfirm.value = { op: 'snapshot.delete', name: vmSnapshotVm.value, snapshot: tag }
}

async function runSnapshotMutation(op: VmCommandOp, name: string, snapshot: string) {
  const reply = await sendVmCommand(op, { name, snapshot }, VM_OP_LABELS[op])
  if (!reply || reply.status !== 'ok') return
  toast.success(`${VM_OP_LABELS[op]}成功`, `虚拟机：${name} · 快照：${snapshot}`)
  if (vmSnapshotOpen.value && vmSnapshotVm.value === name) await refreshVmSnapshots()
}

// ---- 磁盘链 / 导出（阶段 2 再接对象存储上传） ----

async function openVmDiskInfo(name: string) {
  vmDiskVm.value = name
  vmDiskInfo.value = null
  vmDiskOpen.value = true
  const reply = await sendVmCommand('disk.info', { name }, '读取磁盘链')
  if (reply?.status === 'ok') vmDiskInfo.value = parseDiskInfo(reply.data)
}

function refreshVmDiskInfo() {
  if (vmDiskVm.value) void openVmDiskInfo(vmDiskVm.value)
}

async function openVmExport(name: string) {
  vmExportVm.value = name
  vmExportResult.value = null
  vmExportOpen.value = true
  const reply = await sendVmCommand('export', { name }, '导出磁盘')
  if (reply?.status === 'ok') {
    const parsed = parseExportResult(reply.data)
    if (parsed) vmExportResult.value = parsed
    else toast.error('导出结果无法解析', 'agent 没有返回 path/sha256')
  }
}

// ---- 制作镜像（image.make → verify → publish；设计与 agent 契约见 §8.5） ----

const VM_IMAGE_MAKE_POLL_ATTEMPTS = 600
const canMakeImage = computed(() => auth.can(PERMISSIONS.imageManage) && canVmControl.value)

type MakeStepStatus = 'pending' | 'running' | 'ok' | 'error'
interface MakeStep {
  key: string
  label: string
  status: MakeStepStatus
  detail: string
}
interface MakeImageDone {
  imageId: string
  manifestDigest: string
  channel: string
  status: string
}

const vmMakeOpen = ref(false)
const vmMakeVm = ref('')
const vmMakeBusy = ref(false)
const vmMakeError = ref('')
const vmMakeDone = ref<MakeImageDone | null>(null)
const vmMakeSteps = ref<MakeStep[]>([])
const vmMakeForm = ref<VmMakeImageForm>({
  name: '',
  version: '1.0.0',
  platform: 'linux/amd64',
  baseVolume: '',
  description: '',
  channel: 'prod',
})

/** VM 名 → 合法镜像名（小写 + 白名单字符；仅用于预填，可编辑）。 */
function defaultImageName(vmName: string): string {
  const normalized = vmName
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^[._-]+|[._-]+$/g, '')
    .slice(0, 64)
  return normalized || 'vm-image'
}

function setMakeStep(key: string, status: MakeStepStatus, detail = '') {
  vmMakeSteps.value = vmMakeSteps.value.map((step) =>
    step.key === key ? { ...step, status, detail } : step,
  )
}

function openVmMake(vmName: string) {
  vmMakeVm.value = vmName
  vmMakeForm.value = {
    name: defaultImageName(vmName),
    version: '1.0.0',
    platform: 'linux/amd64',
    baseVolume: '',
    description: '',
    channel: 'prod',
  }
  vmMakeError.value = ''
  vmMakeDone.value = null
  vmMakeSteps.value = []
  vmMakeOpen.value = true
}

async function submitVmMake() {
  const error = validateMakeImageForm(vmMakeForm.value)
  vmMakeError.value = error ?? ''
  if (error) return
  vmMakeBusy.value = true
  vmMakeDone.value = null
  vmMakeError.value = ''
  vmMakeSteps.value = [
    {
      key: 'make',
      label: '导出并上传（image.make）',
      status: 'running',
      detail: '等待机器执行，最长 10 分钟',
    },
    { key: 'verify', label: '校验镜像（verify）', status: 'pending', detail: '' },
    { key: 'publish', label: '发布镜像（publish）', status: 'pending', detail: '' },
  ]
  try {
    const reply = await sendVmCommand(
      'image.make',
      buildMakeImageArgs(vmMakeVm.value, vmMakeForm.value),
      '制作镜像',
      VM_IMAGE_MAKE_POLL_ATTEMPTS,
    )
    if (!reply || reply.status !== 'ok') {
      setMakeStep('make', 'error', reply?.error ?? '命令未成功')
      return
    }
    const parsed = parseMakeImageResult(reply.data)
    if (!parsed) {
      setMakeStep('make', 'error', 'agent 没有返回 image_id/manifest_digest')
      return
    }
    setMakeStep('make', 'ok', `image_id=${parsed.imageId}`)

    setMakeStep('verify', 'running', '')
    const verifyReply = await api.verifyImage(parsed.imageId)
    const checks = verifyReply.verification?.report?.checks ?? []
    const failedChecks = checks.filter((check) => check.ok === false)
    if (verifyReply.verification?.ok !== true) {
      const detail =
        verifyReply.verification?.report?.errors?.join('；') ||
        failedChecks
          .map((check) => check.name ?? '')
          .filter(Boolean)
          .join('；') ||
        'verify 未通过'
      setMakeStep('verify', 'error', detail)
      vmMakeError.value = detail
      return
    }
    setMakeStep('verify', 'ok', `${checks.length} 项检查通过`)

    setMakeStep('publish', 'running', '')
    const artifact = await api.publishImage(parsed.imageId, {
      channel: vmMakeForm.value.channel,
    })
    setMakeStep('publish', 'ok', `status=${artifact.status}`)
    vmMakeDone.value = {
      imageId: parsed.imageId,
      manifestDigest: parsed.manifestDigest,
      channel: vmMakeForm.value.channel,
      status: artifact.status,
    }
    toast.success(
      '镜像制作完成',
      `${vmMakeForm.value.name}:${vmMakeForm.value.version} → ${artifact.status}`,
    )
    // 新镜像可能作为底包出现在「新建虚拟机 / 下发环境」下拉里。
    void loadVmBaseImages()
  } catch (caught) {
    const message = errorMessage(caught, '制作镜像失败')
    vmMakeError.value = message
    const running = vmMakeSteps.value.find((step) => step.status === 'running')
    if (running) setMakeStep(running.key, 'error', message)
  } finally {
    vmMakeBusy.value = false
  }
}

async function load(silent = false) {
  if (!serial.value) return
  if (!silent) loading.value = true
  error.value = ''
  notFound.value = false
  try {
    detail.value = await api.getMachine(serial.value, { history_limit: 50 })
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

const telemetryBusy = ref(false)

// 毫秒级实时指标：经 device-pki 内部中转，SSE 持续推流；不请求/不写 DB。
let liveAbort: AbortController | null = null
let liveRetry: ReturnType<typeof setTimeout> | null = null
let liveDisposed = false

function scheduleLiveReconnect() {
  if (liveDisposed) return
  relayStatus.value = 'offline'
  if (liveRetry) clearTimeout(liveRetry)
  liveRetry = setTimeout(() => {
    liveRetry = null
    void connectLiveMetrics()
  }, 2_000)
}

async function connectLiveMetrics() {
  if (liveDisposed || !liveMode.value || !canViewLive.value) return
  liveAbort?.abort()
  const local = new AbortController()
  liveAbort = local
  relayStatus.value = 'connecting'
  try {
    await streamLiveMetrics({
      serial: serial.value,
      signal: local.signal,
      onOpen: () => {
        relayStatus.value = 'live'
      },
      onMetric: (sample) => {
        relayStatus.value = 'live'
        const next = [...liveSamples.value, sample]
        liveSamples.value = next.slice(-240)
      },
      onHeartbeat: () => {
        relayStatus.value = 'live'
      },
    })
    if (!liveDisposed && !local.signal.aborted) scheduleLiveReconnect()
  } catch {
    if (!liveDisposed && !local.signal.aborted) scheduleLiveReconnect()
  }
}

function stopLiveMetrics() {
  if (liveRetry) clearTimeout(liveRetry)
  liveRetry = null
  liveAbort?.abort()
  liveAbort = null
  relayStatus.value = 'offline'
}

watch(liveMode, (enabled) => {
  if (enabled) void connectLiveMetrics()
  else stopLiveMetrics()
})

onUnmounted(() => {
  liveDisposed = true
  stopLiveMetrics()
})

/** 请求 agent 立即上报一次 fast 快照（走 device_downlink，不依赖定时器）。 */
async function poweroffMachine() {
  if (!serial.value) return
  poweroffBusy.value = true
  try {
    await api.poweroffMachine(serial.value)
    poweroffOpen.value = false
    toast.success('关机命令已下发', 'agent 默认禁用电源命令；需在 /etc/qe-agent.env 设置 QE_AGENT_ALLOW_POWEROFF=1')
  } catch (caught) {
    toast.error('关机命令下发失败', errorMessage(caught, '请确认机器在线且有权执行'))
  } finally {
    poweroffBusy.value = false
  }
}

async function requestTelemetryReport() {
  if (!serial.value) return
  telemetryBusy.value = true
  try {
    await api.requestTelemetryReport(serial.value)
    toast.success('已请求立即上报', 'agent 收到后会马上采集，通常 1–2 秒内刷新')
    window.setTimeout(() => void load(true), 1500)
  } catch (caught) {
    toast.error('请求立即上报失败', errorMessage(caught, '请确认机器在线后重试'))
  } finally {
    telemetryBusy.value = false
  }
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

onMounted(() => {
  void load()
  void connectLiveMetrics()
})
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
          <label
            v-if="canViewLive"
            class="flex items-center gap-2 text-[11.5px] text-muted"
            title="通过 device-pki 中转持续接收毫秒级指标（不写数据库）"
          >
            <input
              v-model="liveMode"
              type="checkbox"
              class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
            />
            <span
              class="inline-flex items-center gap-1"
              :class="relayStatus === 'live' ? 'text-emerald-600' : relayStatus === 'offline' ? 'text-rose-600' : 'text-amber-600'"
            >
              <span
                class="h-1.5 w-1.5 rounded-full"
                :class="relayStatus === 'live' ? 'bg-emerald-500 animate-pulse' : relayStatus === 'offline' ? 'bg-rose-500' : 'bg-amber-500'"
              />
              实时指标
            </span>
          </label>
          <AppButton
            v-if="canRequestTelemetry"
            icon="refresh"
            :loading="telemetryBusy"
            title="通过 device_downlink 请求 agent 立即上报一次快照"
            @click="requestTelemetryReport"
          >
            立即上报
          </AppButton>
          <AppButton
            v-if="canRequestTelemetry"
            icon="power"
            variant="danger"
            title="触发 agent 主机系统关机（默认需显式开启）"
            @click="poweroffOpen = true"
          >
            关机
          </AppButton>
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

      <MachineMetricsChart
        v-if="report"
        class="mt-4"
        :points="chartPoints"
        :live-samples="liveSamples"
        :live="liveMode && relayStatus === 'live'"
      />

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-8 flex min-w-0 flex-col gap-4">
          <PageSection
            title="虚拟机"
            description="远程控制：命令经 device_downlink 下发到机器上的 room-agent 执行"
          >
            <div class="flex items-center justify-between gap-2 px-5 pt-4">
              <p class="text-[11px] text-muted">共 {{ vmRows.length }} 台虚拟机</p>
              <div class="flex items-center gap-2">
                <template v-if="canVmControl">
                  <AppButton size="sm" icon="plus" :disabled="vmBusy" @click="openVmCreate">
                    新建虚拟机
                  </AppButton>
                  <AppButton
                    size="sm"
                    icon="refresh"
                    :loading="vmBusy && !vmBusyName"
                    :disabled="vmBusy"
                    @click="refreshVmList"
                  >
                    刷新列表
                  </AppButton>
                </template>
                <span v-else class="text-[10.5px] text-faint">只读（需要 vm:control 权限）</span>
              </div>
            </div>

            <ul v-if="vmRows.length" class="mt-2 divide-y divide-line/70">
              <li
                v-for="vm in vmRows"
                :key="vm.name"
                class="flex flex-wrap items-center justify-between gap-2 px-5 py-3"
              >
                <div class="min-w-0">
                  <p class="truncate font-mono text-[11.5px] text-ink">{{ vm.name }}</p>
                  <p class="mt-0.5 text-[10.5px] text-faint">
                    {{ vm.vcpus ?? '—' }} vCPU · {{ formatBytes(vm.memoryBytes) }}
                  </p>
                </div>
                <div class="flex flex-wrap items-center gap-1.5">
                  <StateBadge
                    :label="workloadStateMeta(vm.state).label"
                    :tone="workloadStateMeta(vm.state).tone"
                  />
                  <template v-if="canVmControl">
                    <AppButton
                      size="sm"
                      icon="play"
                      :disabled="vmBusy"
                      @click="requestVmOp('start', vm.name)"
                    >
                      启动
                    </AppButton>
                    <AppButton
                      size="sm"
                      :disabled="vmBusy"
                      @click="requestVmOp('shutdown', vm.name)"
                    >
                      关机
                    </AppButton>
                    <AppButton size="sm" :disabled="vmBusy" @click="requestVmOp('reboot', vm.name)">
                      重启
                    </AppButton>
                    <AppButton
                      size="sm"
                      variant="danger"
                      :disabled="vmBusy"
                      @click="requestVmOp('destroy', vm.name)"
                    >
                      销毁
                    </AppButton>
                    <AppButton
                      size="sm"
                      variant="danger"
                      :disabled="vmBusy"
                      @click="requestVmOp('delete', vm.name)"
                    >
                      删除
                    </AppButton>
                    <AppButton size="sm" :disabled="vmBusy" @click="screenshotVm(vm.name)">
                      截图
                    </AppButton>
                    <AppButton
                      size="sm"
                      icon="terminal"
                      :disabled="vmBusy || !isVmRunningState(vm.state)"
                      :title="vncRuntimeHint(vm.state)"
                      @click="openVncConsole(vm.name)"
                    >
                      控制台
                    </AppButton>
                    <AppButton size="sm" :disabled="vmBusy" @click="openVmSnapshots(vm.name)">
                      快照
                    </AppButton>
                    <AppButton size="sm" :disabled="vmBusy" @click="openVmDiskInfo(vm.name)">
                      磁盘链
                    </AppButton>
                    <AppButton size="sm" :disabled="vmBusy" @click="openVmExport(vm.name)">
                      导出
                    </AppButton>
                    <AppButton
                      v-if="canMakeImage"
                      size="sm"
                      icon="package"
                      :disabled="vmBusy || isVmRunningState(vm.state)"
                      :title="
                        isVmRunningState(vm.state)
                          ? '请先关机再制作镜像（导出要求 shut off）'
                          : '把该虚拟机导出为镜像并发布'
                      "
                      @click="openVmMake(vm.name)"
                    >
                      制作镜像
                    </AppButton>
                  </template>
                </div>
              </li>
            </ul>
            <EmptyState
              v-else
              title="没有虚拟机"
              description="最新快照未上报 sections.vms（机器未安装/未启用 libvirt，或 agent 未升级）。"
              icon="server"
            />
          </PageSection>

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
        :open="poweroffOpen"
        title="关闭这台机器？"
        description="通过 device_downlink 向 agent 下发 poweroff，由 agent 所在系统执行关机。"
        detail="agent 默认禁用该命令；只有在 /etc/qe-agent.env 设置 QE_AGENT_ALLOW_POWEROFF=1 并重启 daemon 后才会真正关机。"
        confirm-label="确认关机"
        danger
        :loading="poweroffBusy"
        @confirm="poweroffMachine"
        @close="poweroffOpen = false"
      />

      <ConfirmDialog
        :open="vmConfirm !== null"
        title="虚拟机操作"
        :description="
          vmConfirm
            ? `确认对 ${vmConfirm.name}${vmConfirm.snapshot ? ` 的快照 ${vmConfirm.snapshot}` : ''} 执行「${VM_OP_LABELS[vmConfirm.op]}」？`
            : ''
        "
        :detail="vmConfirmDetail"
        :danger="vmConfirm?.op === 'destroy' || vmConfirm?.op === 'snapshot.delete'"
        confirm-label="确认执行"
        :loading="vmBusy"
        @close="vmConfirm = null"
        @confirm="confirmVmOp"
      />

      <AppModal
        :open="vmScreenshot !== null"
        title="虚拟机截图"
        :description="vmScreenshot ? `虚拟机：${vmScreenshot.name}` : ''"
        size="lg"
        @close="vmScreenshot = null"
      >
        <div v-if="vmScreenshot" class="flex justify-center">
          <img
            :src="vmScreenshot.src"
            :alt="`${vmScreenshot.name} 截图`"
            class="max-h-[70vh] w-auto rounded-md border border-line"
          />
        </div>
        <template #footer>
          <AppButton :disabled="vmBusy" @click="vmScreenshot && screenshotVm(vmScreenshot.name)">
            刷新截图
          </AppButton>
          <AppButton variant="primary" @click="vmScreenshot = null">关闭</AppButton>
        </template>
      </AppModal>

      <AppModal
        :open="vmCreateOpen"
        title="新建虚拟机"
        description="用已发布 VM 底包创建 qcow2 差分盘（只读引用底包，只保存增量）"
        size="lg"
        @close="vmCreateOpen = false"
      >
        <div class="grid grid-cols-2 gap-3">
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            名称
            <input
              v-model="vmCreateForm.name"
              placeholder="lab-1"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            系统底包（已发布 vm_disk 镜像）
            <select
              v-model="vmCreateForm.baseVolume"
              :disabled="vmBaseImagesLoading"
              class="h-9 rounded-md border border-line bg-surface px-2 text-[12px] text-ink outline-none focus:border-accent"
            >
              <option value="">空白盘（不使用底包）</option>
              <option
                v-for="option in vmBaseVolumeOptions"
                :key="option.value"
                :value="option.value"
              >
                {{ option.label }}（{{ option.description }}）
              </option>
            </select>
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            vCPU（1-64）
            <input
              v-model.number="vmCreateForm.vcpus"
              type="number"
              min="1"
              max="64"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            内存 MiB（256-262144）
            <input
              v-model.number="vmCreateForm.memoryMiB"
              type="number"
              min="256"
              max="262144"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            磁盘 GiB（1-2048）
            <input
              v-model.number="vmCreateForm.diskGiB"
              type="number"
              min="1"
              max="2048"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            GPU 直通（可选，逗号分隔最多 2 个）
            <input
              v-model="vmCreateForm.gpu"
              placeholder="0000:01:00.0,0000:02:00.0"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="col-span-2 flex items-center gap-2 text-[11.5px] text-muted">
            <input
              v-model="vmCreateForm.start"
              type="checkbox"
              class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
            />
            创建后立即启动
          </label>
        </div>
        <p v-if="vmCreateError" class="mt-2 text-[11px] text-rose-600">{{ vmCreateError }}</p>
        <p v-if="!vmBaseVolumeOptions.length" class="mt-2 text-[10.5px] text-faint">
          没有已发布的 vm_disk 镜像：可在「镜像」页发布 VM 磁盘镜像后再选择底包，或先用空白盘。
        </p>
        <template #footer>
          <AppButton :disabled="vmBusy" @click="vmCreateOpen = false">取消</AppButton>
          <AppButton variant="primary" :loading="vmBusy" @click="submitVmCreate">创建</AppButton>
        </template>
      </AppModal>

      <AppModal
        :open="vmSnapshotOpen"
        title="虚拟机快照"
        :description="
          vmSnapshotVm ? `虚拟机：${vmSnapshotVm}（快照保存在差分盘内部，要求关机状态）` : ''
        "
        size="lg"
        @close="vmSnapshotOpen = false"
      >
        <div class="flex items-end gap-2">
          <label class="flex flex-1 flex-col gap-1 text-[11px] text-muted">
            新快照名
            <input
              v-model="vmSnapshotInput"
              placeholder="snap-1"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <AppButton :disabled="vmBusy" :loading="vmBusy" @click="createVmSnapshot"
            >创建快照</AppButton
          >
          <AppButton :disabled="vmBusy" icon="refresh" @click="refreshVmSnapshots">刷新</AppButton>
        </div>
        <ul
          v-if="vmSnapshots.length"
          class="mt-3 divide-y divide-line/70 rounded-md border border-line"
        >
          <li
            v-for="snapshot in vmSnapshots"
            :key="snapshot.tag"
            class="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
          >
            <div class="min-w-0">
              <p class="truncate font-mono text-[11px] text-ink">{{ snapshot.tag }}</p>
              <p class="mt-0.5 text-[10px] text-faint">
                {{ snapshot.createdAt ?? snapshot.date ?? '—' }}
                <template v-if="snapshot.vmSize"> · {{ snapshot.vmSize }}</template>
              </p>
            </div>
            <div class="flex items-center gap-1.5">
              <AppButton size="sm" :disabled="vmBusy" @click="requestSnapshotRestore(snapshot.tag)">
                恢复
              </AppButton>
              <AppButton
                size="sm"
                variant="danger"
                :disabled="vmBusy"
                @click="requestSnapshotDelete(snapshot.tag)"
              >
                删除
              </AppButton>
            </div>
          </li>
        </ul>
        <p v-else class="mt-3 text-[11px] text-muted">还没有快照（或列表尚未加载）。</p>
      </AppModal>

      <AppModal
        :open="vmDiskOpen"
        title="磁盘链"
        :description="vmDiskVm ? `虚拟机：${vmDiskVm}（底包 → 差分盘 → 快照）` : ''"
        size="lg"
        @close="vmDiskOpen = false"
      >
        <div v-if="vmDiskInfo">
          <ul v-if="vmDiskInfo.chain.length" class="mb-3 flex flex-wrap items-center gap-1.5">
            <li
              v-for="(item, index) in vmDiskInfo.chain"
              :key="`${item}-${index}`"
              class="flex items-center gap-1.5"
            >
              <span
                class="rounded border border-line bg-canvas px-2 py-1 font-mono text-[10px] text-ink-soft"
              >
                {{ item }}
              </span>
              <span v-if="index < vmDiskInfo.chain.length - 1" class="text-faint">→</span>
            </li>
          </ul>
          <table class="w-full text-left text-[10.5px]">
            <thead class="text-faint">
              <tr>
                <th class="py-1">path</th>
                <th class="py-1">format</th>
                <th class="py-1">virtual_size</th>
                <th class="py-1">backing_path</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="disk in vmDiskInfo.disks" :key="disk.path" class="border-t border-line/60">
                <td class="max-w-[260px] truncate py-1 font-mono text-ink-soft">{{ disk.path }}</td>
                <td class="py-1">{{ disk.format ?? '—' }}</td>
                <td class="py-1 tabular-nums">{{ formatBytes(disk.virtualSize) }}</td>
                <td class="max-w-[220px] truncate py-1 font-mono text-faint">
                  {{ disk.backingPath ?? '—' }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-else class="text-[11px] text-muted">正在读取磁盘链（disk.info）…</p>
        <template #footer>
          <AppButton :disabled="vmBusy" icon="refresh" @click="refreshVmDiskInfo">刷新</AppButton>
          <AppButton variant="primary" @click="vmDiskOpen = false">关闭</AppButton>
        </template>
      </AppModal>

      <AppModal
        :open="vmExportOpen"
        title="导出磁盘"
        :description="
          vmExportVm ? `虚拟机：${vmExportVm}（导出为完整 qcow2；上传为镜像 = 阶段 2）` : ''
        "
        size="lg"
        @close="vmExportOpen = false"
      >
        <div v-if="vmExportResult" class="flex flex-col gap-2 text-[11.5px]">
          <div class="rounded-md border border-line bg-canvas/50 px-3 py-2">
            <p class="text-[10.5px] text-faint">导出路径（机器本地）</p>
            <p class="mt-0.5 break-all font-mono text-[11px] text-ink">{{ vmExportResult.path }}</p>
          </div>
          <div class="rounded-md border border-line bg-canvas/50 px-3 py-2">
            <p class="text-[10.5px] text-faint">sha256</p>
            <p class="mt-0.5 break-all font-mono text-[11px] text-ink">
              {{ vmExportResult.sha256 ?? '—' }}
            </p>
          </div>
          <p class="text-[10.5px] text-faint">
            大小：{{ formatBytes(vmExportResult.sizeBytes) }} · “上传到对象存储并发布为底包”将在阶段
            2 接入（当前先产出本地文件）。
          </p>
        </div>
        <p v-else class="text-[11px] text-muted">正在导出磁盘（export，可能需要较长时间）…</p>
        <template #footer>
          <AppButton :disabled="vmBusy" @click="vmExportVm && openVmExport(vmExportVm)"
            >重新导出</AppButton
          >
          <AppButton variant="primary" @click="vmExportOpen = false">关闭</AppButton>
        </template>
      </AppModal>

      <AppModal
        :open="vmMakeOpen"
        title="制作镜像"
        :description="
          vmMakeVm ? `从 ${vmMakeVm} 导出 qcow2 → 分片上传 → verify → publish（虚拟机需关机）` : ''
        "
        size="lg"
        @close="vmMakeOpen = false"
      >
        <div class="grid grid-cols-2 gap-3">
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            镜像名（小写字母/数字/`-_.`）
            <input
              v-model="vmMakeForm.name"
              placeholder="lab-base"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            版本
            <input
              v-model="vmMakeForm.version"
              placeholder="1.0.0"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            平台（{os}/{arch}）
            <input
              v-model="vmMakeForm.platform"
              placeholder="linux/amd64"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="flex flex-col gap-1 text-[11px] text-muted">
            发布通道
            <select
              v-model="vmMakeForm.channel"
              class="h-9 rounded-md border border-line bg-surface px-2 text-[12px] text-ink outline-none focus:border-accent"
            >
              <option value="dev">dev</option>
              <option value="test">test</option>
              <option value="prod">prod</option>
            </select>
          </label>
          <label class="col-span-2 flex flex-col gap-1 text-[11px] text-muted">
            底包卷名（可选，写入 manifest annotations 作来源信息）
            <input
              v-model="vmMakeForm.baseVolume"
              placeholder="ubuntu-22.04（可留空）"
              class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
          <label class="col-span-2 flex flex-col gap-1 text-[11px] text-muted">
            说明（可选）
            <textarea
              v-model="vmMakeForm.description"
              rows="2"
              class="rounded-md border border-line bg-surface px-3 py-2 text-[12px] text-ink outline-none focus:border-accent"
            />
          </label>
        </div>

        <p v-if="vmMakeError" class="mt-2 text-[11px] text-rose-600">{{ vmMakeError }}</p>

        <ol v-if="vmMakeSteps.length" class="mt-3 flex flex-col gap-1.5">
          <li
            v-for="step in vmMakeSteps"
            :key="step.key"
            class="flex items-start gap-2 rounded-md border border-line bg-canvas/40 px-3 py-2 text-[11px]"
          >
            <StateBadge
              :label="
                step.status === 'ok'
                  ? '完成'
                  : step.status === 'error'
                    ? '失败'
                    : step.status === 'running'
                      ? '进行中'
                      : '等待'
              "
              :tone="
                step.status === 'ok'
                  ? 'emerald'
                  : step.status === 'error'
                    ? 'rose'
                    : step.status === 'running'
                      ? 'amber'
                      : 'slate'
              "
            />
            <div class="min-w-0">
              <p class="text-[11px] text-ink">{{ step.label }}</p>
              <p v-if="step.detail" class="mt-0.5 break-all font-mono text-[10px] text-faint">
                {{ step.detail }}
              </p>
            </div>
          </li>
        </ol>

        <div
          v-if="vmMakeDone"
          class="mt-3 rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-[11px] text-emerald-700"
        >
          镜像已发布：<span class="font-mono">{{ vmMakeDone.imageId }}</span> ·
          {{ vmMakeDone.channel }} ·
          <RouterLink to="/images" class="font-medium underline">去镜像列表</RouterLink>
        </div>

        <template #footer>
          <AppButton :disabled="vmMakeBusy" @click="vmMakeOpen = false">关闭</AppButton>
          <AppButton
            variant="primary"
            :loading="vmMakeBusy"
            :disabled="vmMakeBusy || vmMakeDone !== null"
            @click="submitVmMake"
          >
            开始制作
          </AppButton>
        </template>
      </AppModal>

      <VncConsoleModal
        v-if="vncTarget"
        :serial="serial"
        :vm="vncTarget.vm"
        :open="vncTarget !== null"
        @close="vncTarget = null"
      />

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
