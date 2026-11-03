/**
 * 自动建单规则（P1 Wave 1）的纯函数与枚举文案：
 * 信号类型、阈值字段定义/范围、范围（global/area/room）、草稿校验与展示格式化。
 * 契约见 docs/design/ticket-p1-contract.md §2。
 */
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

import { ticketPriorityMeta } from './ticketMeta'

export type TicketRuleSignalType =
  | 'machine_health'
  | 'device_offline'
  | 'schedule_delivery'
  | 'presence_delivery'
  | 'transfer'
  | 'image_verify'

export type TicketRuleScopeType = 'global' | 'area' | 'room'

export const TICKET_RULE_SIGNAL_ORDER: TicketRuleSignalType[] = [
  'machine_health',
  'device_offline',
  'schedule_delivery',
  'presence_delivery',
  'transfer',
  'image_verify',
]

export const TICKET_RULE_SCOPE_ORDER: TicketRuleScopeType[] = ['global', 'area', 'room']

/** 单个阈值输入的元数据（后端 thresholds JSONB 里的键 + 合法范围）。 */
export interface RuleThresholdField {
  key: string
  label: string
  unit: string
  min: number
  max: number
  defaultValue: number
  hint: string
}

export interface RuleSignalMeta {
  label: string
  description: string
  /** 自动建单后落入的工单分类（只用于展示）。 */
  category: string
  tone: BadgeTone
  icon: string
  fields: RuleThresholdField[]
}

const CONSECUTIVE_FIELD: RuleThresholdField = {
  key: 'consecutive',
  label: '连续异常次数',
  unit: '次',
  min: 1,
  max: 10,
  defaultValue: 2,
  hint: '机器连续出现非 ok 快照达到该次数后建单（1–10）。',
}

const OFFLINE_FIELD: RuleThresholdField = {
  key: 'offline_minutes',
  label: '离线时长',
  unit: '分钟',
  min: 1,
  max: 1440,
  defaultValue: 10,
  hint: '设备会话离线超过该时长后建单（1–1440 分钟）。',
}

const LOOKBACK_FIELD: RuleThresholdField = {
  key: 'lookback_hours',
  label: '回看窗口',
  unit: '小时',
  min: 1,
  max: 168,
  defaultValue: 24,
  hint: '只统计最近该时间窗口内失败的记录（1–168 小时）。',
}

export const TICKET_RULE_SIGNAL_META: Record<TicketRuleSignalType, RuleSignalMeta> = {
  machine_health: {
    label: '机器健康异常',
    description: 'room-agent 连续上报非 ok 健康快照时建单。',
    category: '机器健康',
    tone: 'rose',
    icon: 'shield',
    fields: [CONSECUTIVE_FIELD],
  },
  device_offline: {
    label: '设备离线',
    description: '设备 mTLS 会话离线超过阈值时建单。',
    category: '设备离线',
    tone: 'amber',
    icon: 'wifi-off',
    fields: [OFFLINE_FIELD],
  },
  schedule_delivery: {
    label: '课表投递失败',
    description: '课表事件推送到房间失败时建单。',
    category: '推送失败',
    tone: 'violet',
    icon: 'calendar',
    fields: [LOOKBACK_FIELD],
  },
  presence_delivery: {
    label: '在线事件投递失败',
    description: '设备在线事件 webhook 投递失败时建单。',
    category: '推送失败',
    tone: 'violet',
    icon: 'zap',
    fields: [LOOKBACK_FIELD],
  },
  transfer: {
    label: '存储分发失败',
    description: '镜像 / 课表文件分发传输失败或过期时建单。',
    category: '分发失败',
    tone: 'rose',
    icon: 'package',
    fields: [LOOKBACK_FIELD],
  },
  image_verify: {
    label: '镜像验证失败',
    description: '镜像最新一次校验结果为失败时建单。',
    category: '分发失败',
    tone: 'rose',
    icon: 'layers',
    fields: [LOOKBACK_FIELD],
  },
}

export const TICKET_RULE_SCOPE_META: Record<
  TicketRuleScopeType,
  { label: string; description: string }
> = {
  global: { label: '全局', description: '对所有区域 / 房间生效。' },
  area: { label: '按区域', description: '仅对指定区域内的对象生效。' },
  room: { label: '按房间', description: '仅对指定房间生效。' },
}

export function normalizeSignalType(value: string | null | undefined): TicketRuleSignalType | null {
  if (!value) return null
  return TICKET_RULE_SIGNAL_ORDER.includes(value as TicketRuleSignalType)
    ? (value as TicketRuleSignalType)
    : null
}

export function normalizeScopeType(value: string | null | undefined): TicketRuleScopeType {
  return TICKET_RULE_SCOPE_ORDER.includes(value as TicketRuleScopeType)
    ? (value as TicketRuleScopeType)
    : 'global'
}

export function signalMeta(value: string | null | undefined): RuleSignalMeta {
  const key = normalizeSignalType(value)
  if (key) return TICKET_RULE_SIGNAL_META[key]
  return {
    label: value || '未知信号',
    description: '未知的自动建单信号类型。',
    category: '其他',
    tone: 'slate',
    icon: 'alert',
    fields: [],
  }
}

export function thresholdFields(value: string | null | undefined): RuleThresholdField[] {
  return signalMeta(value).fields
}

export function defaultThresholds(value: string | null | undefined): Record<string, number> {
  const result: Record<string, number> = {}
  for (const field of thresholdFields(value)) result[field.key] = field.defaultValue
  return result
}

function toInteger(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return Math.round(value)
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) return Math.round(parsed)
  }
  return null
}

/** 把后端 thresholds（JSONB，可能缺键 / 类型不严）收敛成当前信号定义的数字阈值。 */
export function normalizeThresholds(
  signalType: string | null | undefined,
  raw: unknown,
): Record<string, number> {
  const source =
    raw && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {}
  const result: Record<string, number> = {}
  for (const field of thresholdFields(signalType)) {
    const value = toInteger(source[field.key])
    result[field.key] = value === null ? field.defaultValue : value
  }
  return result
}

/** 只保留当前信号定义的阈值键，并把值收敛成数字，用于提交给后端。 */
export function buildThresholdPayload(
  signalType: string | null | undefined,
  raw: Record<string, number | string | null | undefined> | unknown,
): Record<string, number> {
  return normalizeThresholds(signalType, raw)
}

/** 表格「阈值」列文案，例如「连续 2 次」「回看 24 小时」。 */
export function formatRuleThresholds(signalType: string | null | undefined, raw: unknown): string {
  const fields = thresholdFields(signalType)
  if (!fields.length) return '—'
  const values = normalizeThresholds(signalType, raw)
  return fields
    .map((field) =>
      `${field.label.replace('次数', '').replace('时长', '').replace('窗口', '')} ${values[field.key]} ${field.unit}`.trim(),
    )
    .join(' · ')
}

export function scopeLabel(value: string | null | undefined): string {
  return TICKET_RULE_SCOPE_META[normalizeScopeType(value)].label
}

/** 表格「范围」列：全局 / 区域 · 名称 / 房间 · 名称。 */
export function formatRuleScope(
  scopeType: string | null | undefined,
  scopeId: string | null | undefined,
  names?: { areas?: Record<string, string>; rooms?: Record<string, string> },
): string {
  const type = normalizeScopeType(scopeType)
  if (type === 'global') return '全局'
  const name =
    type === 'area'
      ? scopeId
        ? names?.areas?.[scopeId]
        : undefined
      : scopeId
        ? names?.rooms?.[scopeId]
        : undefined
  const prefix = type === 'area' ? '区域' : '房间'
  return name ? `${prefix} · ${name}` : scopeId ? `${prefix} · ${scopeId.slice(0, 8)}` : prefix
}

export function rulePriorityLabel(priority: string | null | undefined): string {
  return ticketPriorityMeta(priority).label
}

/* -------------------------------- 草稿校验 -------------------------------- */

export interface RuleDraft {
  name: string
  signal_type: string
  priority: string
  scope_type: string
  scope_id: string
  enabled: boolean
  thresholds: Record<string, number | string>
}

export interface RuleDraftErrors {
  name?: string
  signal_type?: string
  priority?: string
  scope_id?: string
  thresholds?: Record<string, string>
}

export const RULE_NAME_MAX_LENGTH = 120

/**
 * 纯校验：名称必填、信号合法、范围 area/room 必须选对象、阈值必须是对应范围内的整数。
 * 返回空对象表示通过。
 */
export function validateRuleDraft(draft: RuleDraft): RuleDraftErrors {
  const errors: RuleDraftErrors = {}

  const name = draft.name.trim()
  if (!name) errors.name = '请输入规则名称'
  else if (name.length > RULE_NAME_MAX_LENGTH) {
    errors.name = `规则名称不能超过 ${RULE_NAME_MAX_LENGTH} 个字符`
  }

  if (!normalizeSignalType(draft.signal_type)) errors.signal_type = '请选择信号类型'
  if (!draft.priority) errors.priority = '请选择优先级'

  const scopeType = normalizeScopeType(draft.scope_type)
  if (scopeType !== 'global' && !draft.scope_id) {
    errors.scope_id = scopeType === 'area' ? '请选择区域' : '请选择房间'
  }

  const thresholdErrors: Record<string, string> = {}
  for (const field of thresholdFields(draft.signal_type)) {
    const value = toInteger(draft.thresholds[field.key])
    if (value === null) {
      thresholdErrors[field.key] = `请输入${field.label}`
    } else if (value < field.min || value > field.max) {
      thresholdErrors[field.key] = `${field.label}需在 ${field.min}–${field.max} ${field.unit}之间`
    }
  }
  if (Object.keys(thresholdErrors).length > 0) errors.thresholds = thresholdErrors

  return errors
}

export function hasRuleErrors(errors: RuleDraftErrors): boolean {
  return Boolean(
    errors.name ||
    errors.signal_type ||
    errors.priority ||
    errors.scope_id ||
    (errors.thresholds && Object.keys(errors.thresholds).length > 0),
  )
}
