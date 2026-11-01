import type { NewEventRule } from '@/api/schema'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export type TriggerKind = NewEventRule['trigger_kind']

export interface TriggerKindOption {
  value: TriggerKind
  label: string
  description: string
  tone: BadgeTone
}

/** 触发类型的事实来源：后端 `crates/domain/src/schedule.rs` 的 TriggerKind。 */
export const TRIGGER_KINDS: TriggerKindOption[] = [
  {
    value: 'occurrence_start',
    label: '课程开始',
    description: '排课实例开始时触发',
    tone: 'sky',
  },
  {
    value: 'occurrence_end',
    label: '课程结束',
    description: '排课实例结束时触发',
    tone: 'violet',
  },
  {
    value: 'occurrence_start_before',
    label: '课前提醒',
    description: '按偏移分钟在课程开始前触发',
    tone: 'amber',
  },
  {
    value: 'daily_time',
    label: '每日定时',
    description: '每天按房间时区的固定钟面时间触发',
    tone: 'emerald',
  },
  {
    value: 'room_occupied',
    label: '房间占用',
    description: '课程开始且此刻没有更早开始的实例时触发',
    tone: 'sky',
  },
  {
    value: 'room_idle',
    label: '房间空闲',
    description: '课程结束且此刻没有其它进行中的实例时触发',
    tone: 'slate',
  },
]

const KIND_MAP = new Map<string, TriggerKindOption>(
  TRIGGER_KINDS.map((option) => [option.value, option]),
)

export function triggerKindLabel(kind: string | null | undefined): string {
  if (!kind) return '—'
  return KIND_MAP.get(kind)?.label ?? kind
}

export function triggerKindDescription(kind: string | null | undefined): string {
  if (!kind) return ''
  return KIND_MAP.get(kind)?.description ?? ''
}

export function triggerKindTone(kind: string | null | undefined): BadgeTone {
  if (!kind) return 'slate'
  return KIND_MAP.get(kind)?.tone ?? 'slate'
}
