import type { BadgeTone } from '@/components/ui/StateBadge.vue'

/** `install_isos.status`（契约 B.1）。 */
export type IsoStatus = 'uploading' | 'ready' | 'failed'

/** 可见范围（契约 C.2：v4 新增；旧数据缺省视为 private）。 */
export type IsoVisibility = 'private' | 'shared'

/** 共享审批状态机：none → pending → approved / rejected（可再次申请）。 */
export type IsoShareStatus = 'none' | 'pending' | 'approved' | 'rejected'

/**
 * ISO 资产（`GET /admin/isos` 列表元素）。
 *
 * v3 只有上传相关字段；v4（附录 C.2）增加共享审批字段。这里把 v4 字段标成
 * 可选，保证服务端灰度/旧版本部署时页面仍能渲染。
 */
export interface IsoAsset {
  id: string
  filename: string
  display_name: string
  description?: string | null
  size_bytes: number
  sha256?: string | null
  status: IsoStatus
  is_preset: boolean
  uploader_user_id?: string | null
  uploader_name?: string | null
  created_at: string
  updated_at: string
  uploaded_at?: string | null
  // ---- v4 共享审批（附录 C.2）----
  visibility?: IsoVisibility
  share_status?: IsoShareStatus
  is_shared?: boolean
  share_note?: string | null
  share_requested_by?: string | null
  share_reviewed_by?: string | null
  share_reviewed_at?: string | null
}

export interface IsoListParams {
  status?: IsoStatus
}

export interface IsoShareReviewInput {
  approve: boolean
  note?: string
}

/** `POST /admin/isos/seed/scan` 响应（契约 B.2）。 */
export interface IsoSeedScanReply {
  scanned: number
  imported: number
  skipped: number
  failed: number
  errors?: string[]
}

interface IsoListEnvelope {
  elements?: unknown
  items?: unknown
}

function isIsoAsset(value: unknown): value is IsoAsset {
  if (!value || typeof value !== 'object') return false
  const record = value as Record<string, unknown>
  return (
    typeof record.id === 'string' &&
    typeof record.filename === 'string' &&
    typeof record.status === 'string'
  )
}

/**
 * 列表响应归一化：v3 是裸数组，v4 契约（B.2/C.2）写的是 `elements[]`；
 * 两种形状都接受，避免服务端灰度期间页面白屏。
 */
export function normalizeIsoList(payload: unknown): IsoAsset[] {
  if (Array.isArray(payload)) return payload.filter(isIsoAsset)

  const envelope = payload as IsoListEnvelope | null | undefined
  const raw = envelope?.elements ?? envelope?.items
  return Array.isArray(raw) ? raw.filter(isIsoAsset) : []
}

/** ISO 状态徽标。 */
export const ISO_STATUS_META: Record<IsoStatus, { label: string; tone: BadgeTone }> = {
  uploading: { label: '上传中', tone: 'amber' },
  ready: { label: '可用', tone: 'emerald' },
  failed: { label: '上传失败', tone: 'rose' },
}

export function isoStatusMeta(status: string): { label: string; tone: BadgeTone } {
  return ISO_STATUS_META[status as IsoStatus] ?? { label: status || '未知', tone: 'slate' }
}

/** 是否已共享（兼容服务端只给 `is_shared` / 只给 visibility+share_status 的实现）。 */
export function isIsoShared(iso: IsoAsset): boolean {
  if (iso.is_shared) return true
  // 契约 C.2：approve 后服务端同时置 visibility=shared + share_status=approved；
  // 只拿到 share_status 的实现也按已共享处理，避免可见性列显示成私有。
  return iso.share_status === 'approved'
}

/**
 * 可见性/共享状态展示（契约 C.2）：
 * 预设 → 公共；pending → 待审批；approved → 已共享；rejected → 已拒绝；其余私有。
 */
export function isoShareMeta(iso: IsoAsset): { label: string; tone: BadgeTone; hint: string } {
  if (iso.is_preset) {
    return { label: '预设', tone: 'accent', hint: '系统预设 ISO，所有教师可用' }
  }
  switch (iso.share_status) {
    case 'pending':
      return { label: '待审批', tone: 'amber', hint: '共享申请等待管理员审批' }
    case 'approved':
      return { label: '已共享', tone: 'emerald', hint: '所有具备 image:read 的教师可用' }
    case 'rejected':
      return {
        label: '已拒绝',
        tone: 'rose',
        hint: iso.share_note || '共享申请被拒绝，可修改后再次申请',
      }
    default:
      if (isIsoShared(iso)) {
        return { label: '已共享', tone: 'emerald', hint: '所有具备 image:read 的教师可用' }
      }
      return { label: '私有', tone: 'slate', hint: '仅上传者与管理员可见' }
  }
}

/** 来源列：预设 / 上传者展示名。 */
export function isoSourceLabel(iso: IsoAsset): string {
  if (iso.is_preset) return '系统预设'
  return iso.uploader_name?.trim() || '未知上传者'
}

/** SHA-256 前 12 位（列表展示用；空值显示 —）。 */
export function shortSha(sha: string | null | undefined): string {
  const value = (sha ?? '').trim()
  return value ? value.slice(0, 12) : '—'
}

/**
 * 「申请时间」：契约没有单独的 share_requested_at 列，pending 申请以
 * `updated_at` 为最近一次状态变更时间；缺失时退回 created_at。
 */
export function isoShareRequestTime(iso: IsoAsset): string {
  return iso.share_reviewed_at || iso.updated_at || iso.created_at
}

/** seed 扫描结果摘要（用于结果条 + toast）。 */
export function seedScanSummary(reply: IsoSeedScanReply): string {
  return `扫描 ${reply.scanned} · 导入 ${reply.imported} · 跳过 ${reply.skipped} · 失败 ${reply.failed}`
}

/** 管理员才能审批/扫描；页面用 roles 判断（接口侧仍由服务端 403 兜底）。 */
export function canReviewShare(isAdmin: boolean, iso: IsoAsset): boolean {
  return isAdmin && iso.share_status === 'pending'
}

/** 是否可删除：非预设（预设由 seed 目录管理）+ 非上传中。 */
export function canDeleteIso(iso: IsoAsset): boolean {
  return !iso.is_preset && iso.status !== 'uploading'
}

/** 是否可撤销共享。 */
export function canUnshareIso(iso: IsoAsset): boolean {
  return !iso.is_preset && isIsoShared(iso)
}
