/**
 * 工单域的中文文案 / 状态色调 / 状态机动作，全部是纯函数或常量，方便单测。
 * 枚举取值见 docs/design/ticket-implementation-contract.md §1（冻结，勿改）。
 */
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import type { TicketCategory, TicketPriority, TicketSource, TicketStatus } from '@/api/schema'
import { formatBytes } from '@/utils/format'

export interface TicketBadgeMeta {
  label: string
  tone: BadgeTone
}

const UNKNOWN_META: TicketBadgeMeta = { label: '未知', tone: 'slate' }

/* -------------------------------- 状态 -------------------------------- */

export const TICKET_STATUS_META: Record<TicketStatus, TicketBadgeMeta> = {
  open: { label: '待处理', tone: 'amber' },
  in_progress: { label: '处理中', tone: 'accent' },
  resolved: { label: '已解决', tone: 'emerald' },
  closed: { label: '已关闭', tone: 'slate' },
  cancelled: { label: '已取消', tone: 'slate' },
}

export const TICKET_STATUS_ORDER: TicketStatus[] = [
  'open',
  'in_progress',
  'resolved',
  'closed',
  'cancelled',
]

export function ticketStatusMeta(status: string | null | undefined): TicketBadgeMeta {
  if (!status) return UNKNOWN_META
  return TICKET_STATUS_META[status as TicketStatus] ?? { label: status, tone: 'slate' }
}

export function isActiveStatus(status: string | null | undefined): boolean {
  return status === 'open' || status === 'in_progress'
}

/** 已关闭 / 已取消的工单锁定评论与状态操作，只能重开。 */
export function isTerminalStatus(status: string | null | undefined): boolean {
  return status === 'closed' || status === 'cancelled'
}

/* -------------------------------- 优先级 ------------------------------- */

export interface TicketPriorityMeta extends TicketBadgeMeta {
  /** SLA 时限（小时），见契约 §3：urgent 1h / high 4h / normal 24h / low 72h。 */
  slaHours: number
  dotClass: string
}

export const TICKET_PRIORITY_META: Record<TicketPriority, TicketPriorityMeta> = {
  urgent: { label: '紧急', tone: 'rose', slaHours: 1, dotClass: 'bg-rose-500' },
  high: { label: '高', tone: 'amber', slaHours: 4, dotClass: 'bg-amber-500' },
  normal: { label: '普通', tone: 'accent', slaHours: 24, dotClass: 'bg-accent' },
  low: { label: '低', tone: 'slate', slaHours: 72, dotClass: 'bg-slate-400' },
}

export const TICKET_PRIORITY_ORDER: TicketPriority[] = ['urgent', 'high', 'normal', 'low']

export function ticketPriorityMeta(priority: string | null | undefined): TicketPriorityMeta {
  if (!priority) return { label: '未知', tone: 'slate', slaHours: 24, dotClass: 'bg-slate-400' }
  return (
    TICKET_PRIORITY_META[priority as TicketPriority] ?? {
      label: priority,
      tone: 'slate',
      slaHours: 24,
      dotClass: 'bg-slate-400',
    }
  )
}

/* -------------------------------- 分类 -------------------------------- */

export interface TicketCategoryMeta extends TicketBadgeMeta {
  /** 图表横条颜色（Tailwind 类名固定，避免运行时拼接）。 */
  barClass: string
}

export const TICKET_CATEGORY_META: Record<TicketCategory, TicketCategoryMeta> = {
  machine_health: { label: '机器健康', tone: 'accent', barClass: 'bg-accent' },
  device_offline: { label: '设备离线', tone: 'amber', barClass: 'bg-amber-500' },
  delivery_failed: { label: '推送失败', tone: 'violet', barClass: 'bg-violet-500' },
  distribution_failed: { label: '分发失败', tone: 'rose', barClass: 'bg-rose-400' },
  security: { label: '安全', tone: 'rose', barClass: 'bg-rose-600' },
  other: { label: '其他', tone: 'slate', barClass: 'bg-slate-400' },
}

export const TICKET_CATEGORY_ORDER: TicketCategory[] = [
  'machine_health',
  'device_offline',
  'delivery_failed',
  'distribution_failed',
  'security',
  'other',
]

export function ticketCategoryMeta(category: string | null | undefined): TicketCategoryMeta {
  if (!category) return { label: '未分类', tone: 'slate', barClass: 'bg-slate-400' }
  return (
    TICKET_CATEGORY_META[category as TicketCategory] ?? {
      label: category,
      tone: 'slate',
      barClass: 'bg-slate-400',
    }
  )
}

export function ticketCategoryLabel(category: string | null | undefined): string {
  return ticketCategoryMeta(category).label
}

/* -------------------------------- 来源 -------------------------------- */

export interface TicketSourceMeta extends TicketBadgeMeta {
  icon: string
}

export const TICKET_SOURCE_META: Record<TicketSource, TicketSourceMeta> = {
  auto: { label: '自动', tone: 'accent', icon: 'spark' },
  manual: { label: '人工', tone: 'slate', icon: 'user' },
}

export function ticketSourceMeta(source: string | null | undefined): TicketSourceMeta {
  if (!source) return { label: '未知', tone: 'slate', icon: 'info' }
  return (
    TICKET_SOURCE_META[source as TicketSource] ?? {
      label: source,
      tone: 'slate',
      icon: 'info',
    }
  )
}

/* ------------------------------- 事件类型 ------------------------------ */

export interface TicketEventKindMeta extends TicketBadgeMeta {
  icon: string
}

export const TICKET_EVENT_KIND_META: Record<string, TicketEventKindMeta> = {
  created: { label: '创建工单', tone: 'accent', icon: 'plus' },
  claimed: { label: '认领工单', tone: 'sky', icon: 'user' },
  assigned: { label: '转派工单', tone: 'violet', icon: 'users' },
  status_changed: { label: '状态变更', tone: 'amber', icon: 'refresh' },
  priority_changed: { label: '优先级变更', tone: 'amber', icon: 'alert' },
  commented: { label: '发表评论', tone: 'slate', icon: 'scroll' },
  updated: { label: '更新内容', tone: 'sky', icon: 'pencil' },
  occurred: { label: '再次触发', tone: 'rose', icon: 'zap' },
}

export function ticketEventKindMeta(kind: string | null | undefined): TicketEventKindMeta {
  if (!kind) return { label: '事件', tone: 'slate', icon: 'info' }
  return TICKET_EVENT_KIND_META[kind] ?? { label: kind, tone: 'slate', icon: 'info' }
}

/** 时间线条目是否是系统写入（自动建单 / 状态机 / 重复触发），与人工评论区分展示。 */
export function isSystemEvent(event: {
  actor_id?: string | null
  actor_name?: string | null
  kind?: string | null
}): boolean {
  if (event.actor_name === '系统') return true
  if (event.kind === 'commented') return false
  return !event.actor_id && !event.actor_name
}

export interface TicketEventLike {
  kind?: string | null
  actor_id?: string | null
  actor_name?: string | null
  from_status?: string | null
  to_status?: string | null
  note?: string | null
  payload?: unknown
}

/**
 * 把一条 ticket_event 转成一句中文描述。纯函数，时间线渲染与单测共用。
 * 注意：不返回 HTML，模板里按纯文本插值（禁止 v-html）。
 */
export function describeTicketEvent(event: TicketEventLike): string {
  const kind = event.kind ?? ''
  const from = event.from_status ? ticketStatusMeta(event.from_status).label : ''
  const to = event.to_status ? ticketStatusMeta(event.to_status).label : ''

  switch (kind) {
    case 'created':
      return '创建了工单'
    case 'claimed':
      return '认领了工单，开始处理'
    case 'assigned':
      return event.note || '变更了处理人'
    case 'status_changed':
      if (from && to) return `状态 ${from} → ${to}`
      return to ? `状态变更为「${to}」` : '变更了状态'
    case 'priority_changed':
      return event.note || '调整了优先级'
    case 'commented':
      // 评论正文在 TicketTimeline 的气泡里单独渲染，这里只给事件摘要，避免同一句话出现两次。
      return '发表了评论'
    case 'updated':
      return '更新了工单内容'
    case 'occurred': {
      const count = readOccurrenceCount(event.payload)
      return count && count > 1 ? `同一问题再次触发（累计 ${count} 次）` : '同一问题再次触发'
    }
    default:
      return event.note || `产生了一条${kind || '工单'}事件`
  }
}

function readOccurrenceCount(payload: unknown): number | null {
  if (!payload || typeof payload !== 'object') return null
  const record = payload as Record<string, unknown>
  const value = record.occurrence_count ?? record.count
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/* ------------------------------- 状态动作 ------------------------------ */

export type TicketActionKey = 'claim' | 'start' | 'resolve' | 'close' | 'reopen' | 'cancel'

export interface TicketAction {
  key: TicketActionKey
  label: string
  icon: string
  variant: 'primary' | 'secondary' | 'danger'
  /** 认领走独立接口（claimTicket），其余走 transitionTicket。 */
  viaClaim?: boolean
  /** transitionTicket 的目标状态。 */
  toStatus?: TicketStatus
  /** 标记解决时必须填写解决说明。 */
  needsResolution?: boolean
  /** 危险 / 不可逆操作走 ConfirmDialog。 */
  needsConfirm?: boolean
}

/**
 * 按当前状态与 ticket:handle 权限推导可用动作。
 * 状态机（契约 §3）：open→in_progress→resolved→closed，open/in_progress→cancelled，
 * resolved/closed/cancelled→open（重开，清空 resolved_at/closed_at）。
 */
export function availableActions(
  status: TicketStatus | string | null | undefined,
  canHandle: boolean,
): TicketAction[] {
  if (!canHandle) return []

  switch (status) {
    case 'open':
      return [
        {
          key: 'start',
          label: '开始处理',
          icon: 'play',
          variant: 'primary',
          toStatus: 'in_progress',
        },
        { key: 'claim', label: '认领', icon: 'user', variant: 'secondary', viaClaim: true },
        { key: 'cancel', label: '取消', icon: 'x', variant: 'danger', needsConfirm: true },
      ]
    case 'in_progress':
      return [
        {
          key: 'resolve',
          label: '标记解决',
          icon: 'check',
          variant: 'primary',
          toStatus: 'resolved',
          needsResolution: true,
        },
        { key: 'cancel', label: '取消', icon: 'x', variant: 'danger', needsConfirm: true },
      ]
    case 'resolved':
      return [
        {
          key: 'close',
          label: '关闭',
          icon: 'check-circle',
          variant: 'primary',
          toStatus: 'closed',
          needsConfirm: true,
        },
        {
          key: 'reopen',
          label: '重开',
          icon: 'rotate-ccw',
          variant: 'secondary',
          toStatus: 'open',
        },
      ]
    case 'closed':
    case 'cancelled':
      return [
        {
          key: 'reopen',
          label: '重开',
          icon: 'rotate-ccw',
          variant: 'secondary',
          toStatus: 'open',
        },
      ]
    default:
      return []
  }
}

/* ------------------------------- 关联上下文 ----------------------------- */

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function readString(source: Record<string, unknown> | null, keys: string[]): string | null {
  if (!source) return null
  for (const key of keys) {
    const value = source[key]
    if (typeof value === 'string' && value.trim()) return value
  }
  return null
}

/** 自动建单的 metadata 里通常带触发时的 health_status，作为上下文缺省值。 */
export function ticketMetadataHealth(metadata: unknown): string | null {
  const record = asRecord(metadata)
  if (!record) return null
  return readString(record, ['health_status', 'machine_health_status'])
}

/* -------------------------------- 根因分类 -------------------------------- */

export type TicketRootCause = 'hardware' | 'network' | 'software' | 'config' | 'usage' | 'other'

export const TICKET_ROOT_CAUSE_ORDER: TicketRootCause[] = [
  'hardware',
  'network',
  'software',
  'config',
  'usage',
  'other',
]

export const TICKET_ROOT_CAUSE_META: Record<
  TicketRootCause,
  { label: string; description: string }
> = {
  hardware: { label: '硬件故障', description: 'GPU、磁盘、内存、网卡等硬件损坏或异常。' },
  network: { label: '网络问题', description: '断网、丢包、DNS、代理或防火墙导致。' },
  software: { label: '软件缺陷', description: 'agent / 服务 / 镜像自身的 bug 或崩溃。' },
  config: { label: '配置错误', description: '房间、课表、阈值、凭据等配置不正确。' },
  usage: { label: '使用方式', description: '用户操作或教室使用方式导致，非系统缺陷。' },
  other: { label: '其他', description: '暂无法归入以上类别的根因。' },
}

export function normalizeRootCause(value: string | null | undefined): TicketRootCause | null {
  if (!value) return null
  return TICKET_ROOT_CAUSE_ORDER.includes(value as TicketRootCause)
    ? (value as TicketRootCause)
    : null
}

export function rootCauseLabel(value: string | null | undefined): string {
  const key = normalizeRootCause(value)
  return key ? TICKET_ROOT_CAUSE_META[key].label : value || '—'
}

/* --------------------------------- 附件 ---------------------------------- */

export const TICKET_ATTACHMENT_MAX_BYTES = 20 * 1024 * 1024
export const TICKET_ATTACHMENT_MAX_FILENAME_LENGTH = 255

/** 后端 content_type 白名单（契约 task-13 §C）。 */
const ATTACHMENT_ALLOWED_TYPES = [
  'image/',
  'application/pdf',
  'text/plain',
  'application/zip',
  'application/gzip',
  'application/octet-stream',
]

export function isAllowedAttachmentType(contentType: string | null | undefined): boolean {
  const value = (contentType || '').toLowerCase().trim()
  if (!value) return false
  return ATTACHMENT_ALLOWED_TYPES.some((prefix) =>
    prefix.endsWith('/') ? value.startsWith(prefix) : value === prefix,
  )
}

/** 上传前的前端校验；返回空字符串表示通过。 */
export function validateAttachmentFile(input: {
  filename: string
  size: number
  contentType: string
}): string {
  const filename = input.filename.trim()
  if (!filename) return '请选择文件'
  if (filename.length > TICKET_ATTACHMENT_MAX_FILENAME_LENGTH) {
    return `文件名不能超过 ${TICKET_ATTACHMENT_MAX_FILENAME_LENGTH} 个字符`
  }
  if (!Number.isFinite(input.size) || input.size <= 0) return '不能上传空文件'
  if (input.size > TICKET_ATTACHMENT_MAX_BYTES) {
    return `单个附件不能超过 ${formatBytes(TICKET_ATTACHMENT_MAX_BYTES)}`
  }
  if (!isAllowedAttachmentType(input.contentType)) {
    return '仅支持图片、PDF、纯文本、ZIP / gzip 或二进制附件'
  }
  return ''
}

export function attachmentSizeLabel(size: number | null | undefined): string {
  if (typeof size !== 'number' || !Number.isFinite(size) || size < 0) return '—'
  return formatBytes(size)
}

/** sha256 必须是 64 位十六进制（crypto.subtle 结果的小写形式）。 */
export function isSha256Hex(value: string | null | undefined): boolean {
  return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)
}

export function attachmentTypeLabel(contentType: string | null | undefined): string {
  const value = (contentType || '').toLowerCase()
  if (value.startsWith('image/')) return '图片'
  if (value === 'application/pdf') return 'PDF'
  if (value === 'text/plain') return '文本'
  if (value === 'application/zip' || value === 'application/gzip') return '压缩包'
  return contentType || '文件'
}
