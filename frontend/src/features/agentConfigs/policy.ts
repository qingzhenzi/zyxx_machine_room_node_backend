import type { AgentConfig, AgentPolicy, AgentPolicyPatch } from '@/api/schema'
import { formatBytes } from '@/utils/format'

export type ScopeType = 'global' | 'area' | 'room'

/** 后端默认生效策略（global → area → room 依次合并的基底）。 */
export const POLICY_DEFAULTS: AgentPolicy = {
  lxc_auto_start: false,
  poll_interval_secs: 2,
  download_concurrency: 4,
  cache_max_bytes: 10 * 1024 * 1024 * 1024,
  telemetry_fast_interval_secs: 10,
  telemetry_full_interval_secs: 300,
}

export const MAX_CACHE_BYTES = 1024 * 1024 * 1024 * 1024

export interface PolicyField {
  key: keyof AgentPolicy
  label: string
  hint: string
  type: 'boolean' | 'number'
  min?: number
  max?: number
  unit?: string
}

export const POLICY_FIELDS: PolicyField[] = [
  {
    key: 'lxc_auto_start',
    label: 'LXC 自动启动',
    hint: '设备上线后自动拉起已分配的 LXC 容器',
    type: 'boolean',
  },
  {
    key: 'poll_interval_secs',
    label: '轮询间隔',
    hint: '1-60 秒，默认 2 秒',
    type: 'number',
    min: 1,
    max: 60,
    unit: '秒',
  },
  {
    key: 'download_concurrency',
    label: '下载并发',
    hint: '1-32，默认 4',
    type: 'number',
    min: 1,
    max: 32,
    unit: '个',
  },
  {
    key: 'cache_max_bytes',
    label: '缓存上限',
    hint: '0-1 TiB，默认 10 GiB',
    type: 'number',
    min: 0,
    max: MAX_CACHE_BYTES,
    unit: '字节',
  },
  {
    key: 'telemetry_fast_interval_secs',
    label: '遥测 fast 间隔',
    hint: '1-60 秒，默认 10 秒（CPU/内存/磁盘轻量快照）',
    type: 'number',
    min: 1,
    max: 60,
    unit: '秒',
  },
  {
    key: 'telemetry_full_interval_secs',
    label: '遥测 full 间隔',
    hint: '60-3600 秒，默认 300 秒（含虚拟机/容器/安全/诊断）',
    type: 'number',
    min: 60,
    max: 3600,
    unit: '秒',
  },
]

export const SCOPE_ORDER: ScopeType[] = ['global', 'area', 'room']

export const SCOPE_LABELS: Record<ScopeType, string> = {
  global: '全局',
  area: '区域',
  room: '房间',
}

export function formatPolicyValue(key: keyof AgentPolicy, value: unknown): string {
  if (key === 'lxc_auto_start') return value ? '开启' : '关闭'
  if (typeof value !== 'number') return '—'
  if (key === 'cache_max_bytes') return formatBytes(value)
  if (key === 'poll_interval_secs') return `${value} 秒`
  if (key === 'telemetry_fast_interval_secs' || key === 'telemetry_full_interval_secs') return `${value} 秒`
  return String(value)
}

/** patch 中显式设置的字段；未设置的字段表示继承上层配置。 */
export function patchEntries(patch: AgentPolicyPatch | undefined): {
  key: keyof AgentPolicy
  value: unknown
}[] {
  if (!patch) return []
  const entries: { key: keyof AgentPolicy; value: unknown }[] = []
  for (const field of POLICY_FIELDS) {
    const value = patch[field.key]
    if (value !== undefined && value !== null) entries.push({ key: field.key, value })
  }
  return entries
}

/** 配置行上展示每个字段：有值显示值，无值显示「继承」。 */
export function patchValue(patch: AgentPolicyPatch | undefined, key: keyof AgentPolicy): string {
  const value = patch?.[key]
  if (value === undefined || value === null) return '继承'
  return formatPolicyValue(key, value)
}

export function scopeLabel(config: AgentConfig): string {
  if (config.scope_type === 'global') return '全局'
  return SCOPE_LABELS[config.scope_type]
}

export function cacheGiB(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`
}
