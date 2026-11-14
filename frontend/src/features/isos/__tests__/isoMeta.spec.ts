import { describe, expect, it } from 'vitest'

import {
  canDeleteIso,
  canUnshareIso,
  isIsoShared,
  isoShareMeta,
  isoShareRequestTime,
  isoSourceLabel,
  isoStatusMeta,
  normalizeIsoList,
  seedScanSummary,
  shortSha,
} from '../types'
import type { IsoAsset } from '../types'

function makeIso(overrides: Partial<IsoAsset> = {}): IsoAsset {
  return {
    id: '11111111-1111-1111-1111-111111111111',
    filename: 'Win10_LTSC_2021.iso',
    display_name: 'Windows 10 LTSC 2021',
    size_bytes: 5_368_709_120,
    sha256: 'abcdef0123456789'.repeat(4),
    status: 'ready',
    is_preset: false,
    uploader_user_id: '22222222-2222-2222-2222-222222222222',
    uploader_name: '李老师',
    created_at: '2026-11-13T10:00:00Z',
    updated_at: '2026-11-14T09:00:00Z',
    ...overrides,
  }
}

describe('normalizeIsoList', () => {
  it('兼容 v3 裸数组与 v4 elements/items 包装', () => {
    const iso = makeIso()
    expect(normalizeIsoList([iso])).toHaveLength(1)
    expect(normalizeIsoList({ elements: [iso] })).toHaveLength(1)
    expect(normalizeIsoList({ items: [iso] })).toHaveLength(1)
  })

  it('空响应/非法元素不会导致页面崩溃', () => {
    expect(normalizeIsoList(null)).toEqual([])
    expect(normalizeIsoList(undefined)).toEqual([])
    expect(normalizeIsoList({})).toEqual([])
    expect(normalizeIsoList([{ id: 'x' }, makeIso()])).toHaveLength(1)
  })
})

describe('ISO 共享/可见性文案', () => {
  it('预设 → 公共；pending/approved/rejected/private 分别映射', () => {
    expect(isoShareMeta(makeIso({ is_preset: true })).label).toBe('预设')
    expect(isoShareMeta(makeIso({ share_status: 'pending' })).label).toBe('待审批')
    expect(isoShareMeta(makeIso({ share_status: 'approved' })).label).toBe('已共享')
    expect(isoShareMeta(makeIso({ share_status: 'rejected' })).label).toBe('已拒绝')
    expect(isoShareMeta(makeIso()).label).toBe('私有')
  })

  it('兼容只给 is_shared 或只给 visibility+share_status 的服务端实现', () => {
    expect(isIsoShared(makeIso({ is_shared: true }))).toBe(true)
    expect(isIsoShared(makeIso({ visibility: 'shared', share_status: 'approved' }))).toBe(true)
    expect(isIsoShared(makeIso({ visibility: 'private' }))).toBe(false)
  })

  it('预设不可删除/不可撤销共享；上传中不可删除', () => {
    expect(canDeleteIso(makeIso({ is_preset: true }))).toBe(false)
    expect(canUnshareIso(makeIso({ is_preset: true }))).toBe(false)
    expect(canDeleteIso(makeIso({ status: 'uploading' }))).toBe(false)
    expect(canDeleteIso(makeIso())).toBe(true)
    expect(canUnshareIso(makeIso({ share_status: 'approved' }))).toBe(true)
  })
})

describe('展示辅助', () => {
  it('来源列：预设优先，其次上传者名', () => {
    expect(isoSourceLabel(makeIso({ is_preset: true }))).toBe('系统预设')
    expect(isoSourceLabel(makeIso())).toBe('李老师')
    expect(isoSourceLabel(makeIso({ uploader_name: null }))).toBe('未知上传者')
  })

  it('SHA 前 12 位与状态徽标容错', () => {
    expect(shortSha('abcdef0123456789'.repeat(4))).toBe('abcdef012345')
    expect(shortSha(null)).toBe('—')
    expect(isoStatusMeta('ready').label).toBe('可用')
    expect(isoStatusMeta('bogus').tone).toBe('slate')
  })

  it('申请时间回退到 updated_at/created_at；扫描结果摘要完整', () => {
    const iso = makeIso({ updated_at: '2026-11-14T09:00:00Z' })
    expect(isoShareRequestTime(iso)).toBe('2026-11-14T09:00:00Z')
    expect(seedScanSummary({ scanned: 3, imported: 1, skipped: 2, failed: 0 })).toContain(
      '扫描 3 · 导入 1 · 跳过 2 · 失败 0',
    )
  })
})
