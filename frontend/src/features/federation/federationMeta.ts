//! 总控联邦页面的纯展示逻辑（状态徽标/计数格式化），便于单测。

import type { FederationCounts } from '@/api/schema'

export type FederationStatusTone = 'emerald' | 'rose' | 'slate'

export interface FederationStatusMeta {
  label: string
  tone: FederationStatusTone
}

/** 节点状态 → 徽标文案/tone。 */
export function nodeStatusMeta(status: string | null | undefined): FederationStatusMeta {
  switch ((status ?? '').toLowerCase()) {
    case 'online':
      return { label: '在线', tone: 'emerald' }
    case 'offline':
      return { label: '离线', tone: 'rose' }
    default:
      return { label: '未刷新', tone: 'slate' }
  }
}

/** 大数字缩写：1234 → 1.2k（联邦 KPI 用，保持短）。 */
export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—'
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`
  return String(value)
}

export interface SummaryNumber {
  label: string
  value: number
}

/** 节点摘要 → 页面展示的四个关键数字。 */
export function summaryNumbers(
  summary: Partial<FederationCounts> | null | undefined,
): SummaryNumber[] {
  return [
    { label: '房间', value: summary?.rooms ?? 0 },
    { label: '机器', value: summary?.machines ?? 0 },
    { label: '在线机器', value: summary?.online_machines ?? 0 },
    { label: '未结束工单', value: summary?.open_tickets ?? 0 },
  ]
}

/** 简易 base_url 前端校验（与后端 normalize_base_url 同一口径的必要子集）。 */
export function validateNodeBaseUrl(raw: string): string | null {
  const value = raw.trim()
  if (!value) return '请填写节点地址'
  if (!/^https?:\/\//i.test(value)) return '节点地址需以 http:// 或 https:// 开头'
  if (/\s/.test(value)) return '节点地址不能包含空格'
  try {
    const url = new URL(value)
    if (url.search || url.hash) return '节点地址不能包含 query 或 fragment'
    if (url.pathname !== '/' && url.pathname !== '') return '节点地址不能包含路径'
  } catch {
    return '节点地址不是合法 URL'
  }
  return null
}
