/**
 * 工单分析的纯函数：按日序列补零、KPI 归一化、MTTR 文案、SLA 超时判断、自动洞察。
 * 不依赖 Vue / DOM，方便 vitest 直接单测。
 */
import { ticketCategoryLabel } from './ticketMeta'

const DAY_MS = 24 * 60 * 60 * 1000

/* ------------------------------- 基础类型 ------------------------------- */

export interface TicketDailyRaw {
  date?: string | null
  created?: number | null
  resolved?: number | null
  closed?: number | null
}

export interface DailySeriesPoint {
  date: string
  created: number
  resolved: number
  closed: number
}

export interface TicketTotalsView {
  created: number
  resolved: number
  closed: number
  open_now: number
  overdue: number
  avg_resolve_secs: number | null
  median_resolve_secs: number | null
}

export interface CategoryCountView {
  category: string
  count: number
}

export interface RoomCountView {
  room_id?: string | null
  room_name?: string | null
  count: number
}

export interface DeviceCountView {
  serial?: string | null
  device_slot?: string | null
  count: number
}

export interface AssigneeLoadView {
  assignee_id?: string | null
  display_name?: string | null
  active: number
  resolved: number
  avg_resolve_secs?: number | null
}

export interface SeriesSummary {
  created: number
  resolved: number
  closed: number
  net: number
}

/* ------------------------------- 日期工具 ------------------------------- */

/** 把 `YYYY-MM-DD` / RFC3339 统一成 `YYYY-MM-DD`；无法解析时返回 null。 */
export function toDayKey(value: string | null | undefined): string | null {
  if (!value) return null
  const text = String(value).trim()
  if (/^\d{4}-\d{2}-\d{2}$/.test(text)) return text
  const time = Date.parse(text)
  if (Number.isNaN(time)) return null
  return new Date(time).toISOString().slice(0, 10)
}

function dayMs(key: string): number {
  return Date.parse(`${key}T00:00:00Z`)
}

function keyFromMs(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10)
}

function toCount(value: number | null | undefined): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.round(value) : 0
}

/**
 * 近 N 天的服务端统计窗口：按 UTC 日对齐（与服务端 `date_trunc('day', ...)` 的口径一致），
 * 保证返回的 by_day 恰好是 N 个日桶。
 */
export function buildStatsRange(
  days: number,
  now: Date = new Date(),
): { from: string; to: string } {
  const safeDays = Math.max(1, Math.floor(days))
  const endKey = now.toISOString().slice(0, 10)
  const startMs = dayMs(endKey) - (safeDays - 1) * DAY_MS
  return { from: new Date(startMs).toISOString(), to: now.toISOString() }
}

export function isOverdue(
  ticket: { status?: string | null; due_at?: string | null },
  nowMs: number = Date.now(),
): boolean {
  if (ticket.status !== 'open' && ticket.status !== 'in_progress') return false
  if (!ticket.due_at) return false
  const due = Date.parse(ticket.due_at)
  if (Number.isNaN(due)) return false
  return due < nowMs
}

/* ------------------------------- 按日序列 ------------------------------- */

/**
 * 把服务端 `by_day` 补成连续日序列；缺失日期填 0。
 * 日期窗口取自 stats.from / stats.to（UTC 日），若数据点落在窗口外则自动扩展，避免丢数。
 */
export function buildDailySeries(
  points: TicketDailyRaw[] | null | undefined,
  from: string | null | undefined,
  to: string | null | undefined,
): DailySeriesPoint[] {
  const bucket = new Map<string, DailySeriesPoint>()

  for (const point of points ?? []) {
    const key = toDayKey(point.date)
    if (!key) continue
    const current = bucket.get(key) ?? { date: key, created: 0, resolved: 0, closed: 0 }
    current.created += toCount(point.created)
    current.resolved += toCount(point.resolved)
    current.closed += toCount(point.closed)
    bucket.set(key, current)
  }

  const pointKeys = [...bucket.keys()].sort()
  const fromKey = toDayKey(from)
  const toKey = toDayKey(to)

  let startKey = fromKey ?? pointKeys[0] ?? null
  let endKey = toKey ?? pointKeys[pointKeys.length - 1] ?? null
  if (!startKey || !endKey) return sortSeries([...bucket.values()])

  const firstPoint = pointKeys[0]
  const lastPoint = pointKeys[pointKeys.length - 1]
  if (firstPoint && firstPoint < startKey) startKey = firstPoint
  if (lastPoint && lastPoint > endKey) endKey = lastPoint

  const startMs = dayMs(startKey)
  const endMs = dayMs(endKey)
  if (Number.isNaN(startMs) || Number.isNaN(endMs) || endMs < startMs) {
    return sortSeries([...bucket.values()])
  }

  const series: DailySeriesPoint[] = []
  for (let ms = startMs, guard = 0; ms <= endMs && guard < 1_000; ms += DAY_MS, guard += 1) {
    const key = keyFromMs(ms)
    series.push(bucket.get(key) ?? { date: key, created: 0, resolved: 0, closed: 0 })
  }
  return series
}

function sortSeries(series: DailySeriesPoint[]): DailySeriesPoint[] {
  return series.sort((left, right) => left.date.localeCompare(right.date))
}

export function summarizeSeries(series: DailySeriesPoint[]): SeriesSummary {
  let created = 0
  let resolved = 0
  let closed = 0
  for (const point of series) {
    created += point.created
    resolved += point.resolved
    closed += point.closed
  }
  return { created, resolved, closed, net: created - resolved }
}

/* ------------------------------- KPI 归一化 ------------------------------ */

export function normalizeTotals(
  raw:
    | {
        created?: number | null
        resolved?: number | null
        closed?: number | null
        open_now?: number | null
        overdue?: number | null
        avg_resolve_secs?: number | null
        median_resolve_secs?: number | null
      }
    | null
    | undefined,
): TicketTotalsView {
  return {
    created: toCount(raw?.created),
    resolved: toCount(raw?.resolved),
    closed: toCount(raw?.closed),
    open_now: toCount(raw?.open_now),
    overdue: toCount(raw?.overdue),
    avg_resolve_secs: toSeconds(raw?.avg_resolve_secs),
    median_resolve_secs: toSeconds(raw?.median_resolve_secs),
  }
}

function toSeconds(value: number | null | undefined): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return null
  return value
}

/** 分类横条数据：归一化 + 按数量倒序（数量相同按契约顺序）。 */
export function toCategoryRows(
  items: { category?: string | null; count?: number | null }[] | null | undefined,
): CategoryCountView[] {
  return (items ?? [])
    .map((item) => ({ category: item.category ?? 'other', count: toCount(item.count) }))
    .filter((item) => item.count > 0)
    .sort((left, right) => right.count - left.count)
}

export function toAssigneeRows(
  items:
    | {
        assignee_id?: string | null
        display_name?: string | null
        active?: number | null
        resolved?: number | null
        avg_resolve_secs?: number | null
      }[]
    | null
    | undefined,
): AssigneeLoadView[] {
  return (items ?? [])
    .map((item) => ({
      assignee_id: item.assignee_id ?? null,
      display_name: item.display_name ?? null,
      active: toCount(item.active),
      resolved: toCount(item.resolved),
      avg_resolve_secs: toSeconds(item.avg_resolve_secs),
    }))
    .filter((item) => item.active + item.resolved > 0)
    .sort((left, right) => right.active + right.resolved - (left.active + left.resolved))
}

/** 百分比（0–100 整数），分母为 0 时返回 0。 */
export function percentage(part: number, total: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(total) || total <= 0) return 0
  return Math.max(0, Math.min(100, Math.round((part / total) * 100)))
}

export function maxCount(items: { count: number }[]): number {
  let max = 0
  for (const item of items) max = Math.max(max, item.count)
  return Math.max(1, max)
}

/* ------------------------------- MTTR 文案 ------------------------------- */

/** 平均处理时长文案；null / 负数返回 `—`。 */
export function formatMttr(seconds: number | null | undefined): string {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return '—'
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} 秒`
  const minutes = seconds / 60
  if (minutes < 60) return `${Math.round(minutes)} 分钟`
  const hours = minutes / 60
  if (hours < 24) return `${hours.toFixed(1)} 小时`
  const days = hours / 24
  return `${days.toFixed(1)} 天`
}

/* ------------------------------- 自动洞察 ------------------------------- */

export interface TicketInsightInput {
  days: number
  totals: TicketTotalsView
  byCategory: CategoryCountView[]
  topRooms: RoomCountView[]
}

export interface TicketInsight {
  title: string
  text: string
}

/**
 * 从同一份 stats 推导一句洞察：最集中的分类 / 房间，给出可执行建议。
 * 纯文本，禁止 HTML。
 */
export function buildInsight(input: TicketInsightInput): TicketInsight {
  const { days, totals } = input
  const topCategory = input.byCategory[0]
  const topRoom = input.topRooms[0]

  if (totals.created <= 0) {
    return {
      title: '自动洞察',
      text: `近 ${days} 天暂无新增工单，暂无法判断故障聚集情况。`,
    }
  }

  const categoryPart = topCategory
    ? `${ticketCategoryLabel(topCategory.category)}类最多（${topCategory.count} 张，占 ${percentage(
        topCategory.count,
        totals.created,
      )}%）`
    : '分类分布暂无数据'

  if (topRoom && topRoom.count >= 2) {
    const roomName = topRoom.room_name || '未命名房间'
    return {
      title: '重复故障提醒',
      text: `近 ${days} 天新增 ${totals.created} 张工单，${roomName} 以 ${topRoom.count} 张居首；全部新增中${categoryPart}，建议优先排查该对象的重复告警。`,
    }
  }

  const density =
    topRoom && topRoom.room_name ? `${topRoom.room_name} 告警相对集中（${topRoom.count} 张），` : ''
  return {
    title: '自动洞察',
    text: `近 ${days} 天新增 ${totals.created} 张工单，${categoryPart}；${density}整体分布较分散，建议持续观察高频对象的告警变化。`,
  }
}
