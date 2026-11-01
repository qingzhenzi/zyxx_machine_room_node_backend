import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export type AnnouncementStatus = 'draft' | 'published' | 'archived'

export interface StatusMeta {
  label: string
  tone: BadgeTone
}

const STATUS_META: Record<string, StatusMeta> = {
  draft: { label: '草稿', tone: 'amber' },
  published: { label: '已发布', tone: 'emerald' },
  archived: { label: '已归档', tone: 'slate' },
}

export function statusMeta(status: string | undefined): StatusMeta {
  if (!status) return { label: '—', tone: 'slate' }
  return STATUS_META[status] ?? { label: status, tone: 'slate' }
}

export const STATUS_FILTER_TABS = [
  { value: '', label: '全部' },
  { value: 'draft', label: '草稿' },
  { value: 'published', label: '已发布' },
  { value: 'archived', label: '已归档' },
]
