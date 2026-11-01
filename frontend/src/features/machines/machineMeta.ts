import type { BadgeTone } from '@/components/ui/StateBadge.vue'

/** 机器总览里各处复用的状态文案/色调，避免在模板里散落映射。 */
export interface MachineStatusMeta {
  label: string
  tone: BadgeTone
}

const HEALTH_META: Record<string, MachineStatusMeta> = {
  ok: { label: '正常', tone: 'emerald' },
  warning: { label: '告警', tone: 'amber' },
  critical: { label: '严重', tone: 'rose' },
  unknown: { label: '未知', tone: 'slate' },
}

/** 设备从未上报过快照（report === null）。 */
export const MACHINE_UNREPORTED_META: MachineStatusMeta = { label: '未上报', tone: 'slate' }

export function machineHealthMeta(status: string | null | undefined): MachineStatusMeta {
  if (!status) return MACHINE_UNREPORTED_META
  return HEALTH_META[status] ?? { label: status, tone: 'slate' }
}

/** 列表筛选下拉：与后端 `health` 查询参数一致。 */
export const MACHINE_HEALTH_FILTERS: { value: string; label: string }[] = [
  { value: 'ok', label: '正常' },
  { value: 'warning', label: '告警' },
  { value: 'critical', label: '严重' },
  { value: 'missing', label: '未上报' },
]

/** 指标进度条色调：与服务端健康阈值（85% 告警 / 95% 严重）相近。 */
export function usageTone(
  percent: number | null | undefined,
): 'emerald' | 'amber' | 'rose' | 'accent' {
  if (percent === null || percent === undefined) return 'accent'
  if (percent >= 95) return 'rose'
  if (percent >= 85) return 'amber'
  if (percent >= 60) return 'accent'
  return 'emerald'
}

/** 健康分大字颜色：90+ 绿、70+ 琥珀、更低玫红。 */
export function healthScoreClass(score: number | null | undefined): string {
  if (score === null || score === undefined) return 'text-faint'
  if (score >= 90) return 'text-emerald-600'
  if (score >= 70) return 'text-amber-600'
  return 'text-rose-600'
}

/** 健康问题严重级别（兼容 info / warning / critical 及常见别名）。 */
export function severityMeta(severity: string | null | undefined): MachineStatusMeta {
  switch ((severity ?? '').toLowerCase()) {
    case 'critical':
    case 'fatal':
    case 'crit':
      return { label: '严重', tone: 'rose' }
    case 'warning':
    case 'warn':
      return { label: '告警', tone: 'amber' }
    case 'info':
    case 'low':
      return { label: '提示', tone: 'sky' }
    default:
      return { label: severity || '未知', tone: 'slate' }
  }
}

/** 安全核查项状态（qeaudit：pass / warn / fail / skip）。 */
export function securityStatusMeta(status: string | null | undefined): MachineStatusMeta {
  switch ((status ?? '').toLowerCase()) {
    case 'pass':
    case 'passed':
    case 'ok':
      return { label: '通过', tone: 'emerald' }
    case 'warn':
    case 'warning':
      return { label: '告警', tone: 'amber' }
    case 'fail':
    case 'failed':
    case 'error':
      return { label: '失败', tone: 'rose' }
    case 'skip':
    case 'skipped':
      return { label: '跳过', tone: 'slate' }
    default:
      return { label: status || '未知', tone: 'slate' }
  }
}

/** 安全核查风险等级（qeaudit：info / low / medium / high / critical）。 */
export function securitySeverityMeta(severity: string | null | undefined): MachineStatusMeta {
  switch ((severity ?? '').toLowerCase()) {
    case 'critical':
      return { label: '严重', tone: 'rose' }
    case 'high':
      return { label: '高', tone: 'rose' }
    case 'medium':
      return { label: '中', tone: 'amber' }
    case 'low':
      return { label: '低', tone: 'sky' }
    case 'info':
      return { label: '提示', tone: 'slate' }
    default:
      return { label: severity || '未知', tone: 'slate' }
  }
}

/** 诊断维度（platform-diagnose：app / system / host / network）。 */
export function diagnoseDimensionMeta(dimension: string | null | undefined): MachineStatusMeta {
  switch ((dimension ?? '').toLowerCase()) {
    case 'app':
      return { label: '应用', tone: 'violet' }
    case 'system':
      return { label: '系统', tone: 'amber' }
    case 'host':
      return { label: '主机', tone: 'rose' }
    case 'network':
      return { label: '网络', tone: 'sky' }
    default:
      return { label: dimension || '未知', tone: 'slate' }
  }
}

/** 容器/虚拟机运行状态色调。 */
export function workloadStateMeta(state: string | null | undefined): MachineStatusMeta {
  switch ((state ?? '').toLowerCase()) {
    case 'running':
      return { label: state ?? 'running', tone: 'emerald' }
    case 'paused':
    case 'restarting':
      return { label: state ?? 'paused', tone: 'amber' }
    case 'exited':
    case 'dead':
    case 'shut off':
    case 'crashed':
      return { label: state ?? 'exited', tone: 'rose' }
    default:
      return { label: state || '未知', tone: 'slate' }
  }
}

/** 表格迷你进度条色调（与 usageTone 一致，但按更宽松阈值给颜色）。 */
export function miniUsageTone(
  percent: number | null | undefined,
): 'emerald' | 'amber' | 'rose' | 'accent' {
  if (percent === null || percent === undefined) return 'accent'
  if (percent >= 90) return 'rose'
  if (percent >= 75) return 'amber'
  return 'emerald'
}
