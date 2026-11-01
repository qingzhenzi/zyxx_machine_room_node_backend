import type { DeviceOnlineState, DeviceStatus } from '@/api/schema'

/** 与 StateBadge 的 tone 保持一致，避免在页面里散落状态映射。 */
export type StatusTone = 'emerald' | 'amber' | 'rose' | 'slate' | 'sky' | 'violet' | 'accent'

interface StatusMeta {
  label: string
  tone: StatusTone
}

export const DEVICE_STATUS_META: Record<DeviceStatus, StatusMeta> = {
  pending: { label: '待处理', tone: 'amber' },
  issued: { label: '已签发', tone: 'sky' },
  active: { label: '已激活', tone: 'emerald' },
  rejected: { label: '已拒绝', tone: 'rose' },
  revoked: { label: '已吊销', tone: 'slate' },
}

export const DEVICE_STATUS_FILTERS: { value: DeviceStatus; label: string }[] = [
  { value: 'pending', label: '待处理' },
  { value: 'issued', label: '已签发' },
  { value: 'active', label: '已激活' },
  { value: 'rejected', label: '已拒绝' },
  { value: 'revoked', label: '已吊销' },
]

export const DEVICE_ONLINE_META: Record<DeviceOnlineState, StatusMeta> = {
  online: { label: '在线', tone: 'emerald' },
  offline: { label: '离线', tone: 'rose' },
}

/** 注册状态；未注册（房间刚签发 token）时给一个中性提示。 */
export function deviceStatusMeta(status: DeviceStatus | null | undefined): StatusMeta {
  if (!status) return { label: '待注册', tone: 'amber' }
  return DEVICE_STATUS_META[status]
}

/** 在线状态；尚未上报过心跳时是未知。 */
export function deviceOnlineMeta(online: DeviceOnlineState | null | undefined): StatusMeta {
  if (!online) return { label: '未知', tone: 'slate' }
  return DEVICE_ONLINE_META[online]
}
