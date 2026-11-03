/**
 * 巡检计划（P1 Wave 3）的纯函数：范围 / 运行状态文案与 tone、间隔格式化、
 * 下次运行时间、耗时与草稿校验。不依赖 Vue / DOM，便于 vitest 直接单测。
 */
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { formatDuration } from '@/utils/format'

export type InspectionScopeType = 'global' | 'area' | 'room'
export type InspectionRunStatus = 'running' | 'ok' | 'issues' | 'failed'

export const INSPECTION_INTERVAL_MIN = 1
export const INSPECTION_INTERVAL_MAX = 168
export const INSPECTION_INTERVAL_DEFAULT = 24

export const INSPECTION_SCOPE_ORDER: InspectionScopeType[] = ['global', 'area', 'room']

export const INSPECTION_SCOPE_META: Record<
  InspectionScopeType,
  { label: string; description: string }
> = {
  global: { label: '全局', description: '巡检全部已注册机器。' },
  area: { label: '按区域', description: '只巡检指定区域内的机器。' },
  room: { label: '按房间', description: '只巡检指定房间的机器。' },
}

export const INSPECTION_RUN_STATUS_META: Record<
  InspectionRunStatus,
  { label: string; tone: BadgeTone; description: string }
> = {
  running: { label: '运行中', tone: 'sky', description: '正在采集机器快照与健康结论。' },
  ok: { label: '正常', tone: 'emerald', description: '巡检完成，范围内机器全部健康。' },
  issues: { label: '有异常', tone: 'amber', description: '巡检完成，范围内存在异常机器。' },
  failed: { label: '失败', tone: 'rose', description: '巡检执行失败，可查看错误信息后重试。' },
}

export function normalizeInspectionScope(value: string | null | undefined): InspectionScopeType {
  return INSPECTION_SCOPE_ORDER.includes(value as InspectionScopeType)
    ? (value as InspectionScopeType)
    : 'global'
}

export function normalizeRunStatus(value: string | null | undefined): InspectionRunStatus | null {
  if (!value) return null
  return value in INSPECTION_RUN_STATUS_META ? (value as InspectionRunStatus) : null
}

export function runStatusMeta(value: string | null | undefined): {
  label: string
  tone: BadgeTone
  description: string
} {
  const key = normalizeRunStatus(value)
  if (!key) return { label: value || '未知', tone: 'slate', description: '未知的巡检运行状态。' }
  return INSPECTION_RUN_STATUS_META[key]
}

export function inspectionScopeLabel(value: string | null | undefined): string {
  return INSPECTION_SCOPE_META[normalizeInspectionScope(value)].label
}

/** 表格「范围」列：全局 / 区域 · 名称 / 房间 · 名称。 */
export function formatInspectionScope(
  scopeType: string | null | undefined,
  scopeId: string | null | undefined,
  names?: { areas?: Record<string, string>; rooms?: Record<string, string> },
): string {
  const type = normalizeInspectionScope(scopeType)
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

/** 间隔文案：1 小时 / 每 6 小时 / 每天 / 每 3 天 / 每 7 天。 */
export function formatInspectionInterval(hours: number | null | undefined): string {
  if (typeof hours !== 'number' || !Number.isFinite(hours) || hours < 1) return '—'
  if (hours === 1) return '每小时'
  if (hours === 24) return '每天'
  if (hours === 168) return '每 7 天'
  if (hours % 24 === 0) return `每 ${hours / 24} 天`
  return `每 ${hours} 小时`
}

/** 下次运行时间：`—` / `等待执行` / `N 分钟后` / `N 小时后` / `N 天后`。 */
export function formatNextRun(
  nextRunAt: string | null | undefined,
  nowMs: number = Date.now(),
): string {
  if (!nextRunAt) return '—'
  const next = Date.parse(nextRunAt)
  if (Number.isNaN(next)) return '—'
  const diff = next - nowMs
  if (diff <= 0) return '等待执行'
  const minutes = diff / 60_000
  if (minutes < 60) return `${Math.max(1, Math.round(minutes))} 分钟后`
  const hours = minutes / 60
  if (hours < 24) return `${Math.round(hours)} 小时后`
  return `${(hours / 24).toFixed(1)} 天后`
}

/** 运行耗时；未结束返回「运行中」。 */
export function formatRunDuration(
  startedAt: string | null | undefined,
  finishedAt: string | null | undefined,
): string {
  if (!startedAt) return '—'
  if (!finishedAt) return '运行中'
  const start = Date.parse(startedAt)
  const end = Date.parse(finishedAt)
  if (Number.isNaN(start) || Number.isNaN(end) || end < start) return '—'
  return formatDuration(end - start)
}

/** 运行历史里「关联工单」列的跳转目标（只接受 uuid）。 */
export function runTicketLink(ticketId: string | null | undefined): string | null {
  if (!ticketId) return null
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(ticketId)
    ? ticketId
    : null
}

/* -------------------------------- 草稿校验 -------------------------------- */

export interface InspectionDraft {
  name: string
  scope_type: string
  scope_id: string
  interval_hours: number | string
  enabled: boolean
}

export interface InspectionDraftErrors {
  name?: string
  scope_id?: string
  interval_hours?: string
}

export const INSPECTION_NAME_MAX_LENGTH = 100

export function validateInspectionDraft(draft: InspectionDraft): InspectionDraftErrors {
  const errors: InspectionDraftErrors = {}

  const name = draft.name.trim()
  if (!name) errors.name = '请输入计划名称'
  else if (name.length > INSPECTION_NAME_MAX_LENGTH) {
    errors.name = `计划名称不能超过 ${INSPECTION_NAME_MAX_LENGTH} 个字符`
  }

  const scopeType = normalizeInspectionScope(draft.scope_type)
  if (scopeType !== 'global' && !draft.scope_id) {
    errors.scope_id = scopeType === 'area' ? '请选择区域' : '请选择房间'
  }

  const interval = Number(draft.interval_hours)
  if (
    !Number.isFinite(interval) ||
    !Number.isInteger(interval) ||
    interval < INSPECTION_INTERVAL_MIN ||
    interval > INSPECTION_INTERVAL_MAX
  ) {
    errors.interval_hours = `间隔需为 ${INSPECTION_INTERVAL_MIN}–${INSPECTION_INTERVAL_MAX} 之间的整数小时`
  }

  return errors
}

export function hasInspectionErrors(errors: InspectionDraftErrors): boolean {
  return Boolean(errors.name || errors.scope_id || errors.interval_hours)
}
