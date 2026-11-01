import type { CollectionStatus, TransferStatus } from '@/api/schema'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export type ProgressTone = 'accent' | 'emerald' | 'amber' | 'rose' | 'line'

export const TRANSFER_STATUS_META: Record<
  TransferStatus,
  { label: string; tone: BadgeTone; progress: ProgressTone }
> = {
  pending: { label: '等待中', tone: 'slate', progress: 'line' },
  running: { label: '进行中', tone: 'amber', progress: 'amber' },
  completed: { label: '已完成', tone: 'emerald', progress: 'emerald' },
  failed: { label: '失败', tone: 'rose', progress: 'rose' },
  expired: { label: '已过期', tone: 'slate', progress: 'line' },
}

export const TRANSFER_DIRECTION_LABELS: Record<'download' | 'upload', string> = {
  download: '下载',
  upload: '上传',
}

export const COLLECTION_STATUS_META: Record<CollectionStatus, { label: string; tone: BadgeTone }> =
  {
    pending: { label: '待上传', tone: 'slate' },
    uploaded: { label: '已上传', tone: 'sky' },
    verified: { label: '已校验', tone: 'emerald' },
    rejected: { label: '已拒绝', tone: 'rose' },
  }

const TRANSFER_FALLBACK = {
  label: '—',
  tone: 'slate' as BadgeTone,
  progress: 'line' as ProgressTone,
}
const COLLECTION_FALLBACK = { label: '—', tone: 'slate' as BadgeTone }

export function transferStatusMeta(value: TransferStatus | null | undefined) {
  return value ? TRANSFER_STATUS_META[value] : TRANSFER_FALLBACK
}

export function transferDirectionLabel(value: 'download' | 'upload' | null | undefined): string {
  return value ? TRANSFER_DIRECTION_LABELS[value] : '—'
}

export function collectionStatusMeta(value: CollectionStatus | null | undefined) {
  return value ? COLLECTION_STATUS_META[value] : COLLECTION_FALLBACK
}

/** 只有 failed / expired 可以重试（与后端一致）。 */
export function canRetryTransfer(status: TransferStatus | null | undefined): boolean {
  return status === 'failed' || status === 'expired'
}

export const COLLECTION_KIND_LABELS: Record<string, string> = {
  logs: '日志',
  log: '日志',
  screenshot: '截图',
  file: '文件',
  artifact: '产物',
}

export function collectionKindLabel(kind: string | null | undefined): string {
  if (!kind) return '—'
  return COLLECTION_KIND_LABELS[kind] ?? kind
}
