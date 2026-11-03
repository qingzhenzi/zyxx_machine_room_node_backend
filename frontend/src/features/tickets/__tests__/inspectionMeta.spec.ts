import { describe, expect, it } from 'vitest'

import {
  formatInspectionInterval,
  formatInspectionScope,
  formatNextRun,
  formatRunDuration,
  hasInspectionErrors,
  inspectionScopeLabel,
  normalizeInspectionScope,
  normalizeRunStatus,
  runStatusMeta,
  runTicketLink,
  validateInspectionDraft,
} from '../inspectionMeta'

const NOW = Date.parse('2026-11-02T12:00:00Z')

describe('巡检范围 / 状态文案', () => {
  it('范围归一化与中文 labels', () => {
    expect(normalizeInspectionScope('area')).toBe('area')
    expect(normalizeInspectionScope('room')).toBe('room')
    expect(normalizeInspectionScope('weird')).toBe('global')
    expect(normalizeInspectionScope(null)).toBe('global')
    expect(inspectionScopeLabel('global')).toBe('全局')
    expect(inspectionScopeLabel('room')).toBe('按房间')
  })

  it('运行状态文案与 tone', () => {
    expect(normalizeRunStatus('ok')).toBe('ok')
    expect(normalizeRunStatus('failed')).toBe('failed')
    expect(normalizeRunStatus('done')).toBeNull()
    expect(runStatusMeta('issues')).toMatchObject({ label: '有异常', tone: 'amber' })
    expect(runStatusMeta('running').tone).toBe('sky')
    expect(runStatusMeta('failed').tone).toBe('rose')
    expect(runStatusMeta('weird')).toMatchObject({ label: 'weird', tone: 'slate' })
  })
})

describe('范围 / 间隔格式化', () => {
  it('范围内嵌名称，缺失时回退短 id', () => {
    expect(formatInspectionScope('global', null)).toBe('全局')
    expect(
      formatInspectionScope('area', 'aaaaaaaa-1111-2222-3333-444444444444', {
        areas: { 'aaaaaaaa-1111-2222-3333-444444444444': '教学楼 A' },
      }),
    ).toBe('区域 · 教学楼 A')
    expect(formatInspectionScope('room', 'bbbbbbbb-1111-2222-3333-444444444444')).toBe(
      '房间 · bbbbbbbb',
    )
    expect(formatInspectionScope('area', null)).toBe('区域')
  })

  it('间隔小时转中文', () => {
    expect(formatInspectionInterval(1)).toBe('每小时')
    expect(formatInspectionInterval(6)).toBe('每 6 小时')
    expect(formatInspectionInterval(24)).toBe('每天')
    expect(formatInspectionInterval(48)).toBe('每 2 天')
    expect(formatInspectionInterval(168)).toBe('每 7 天')
    expect(formatInspectionInterval(null)).toBe('—')
    expect(formatInspectionInterval(0)).toBe('—')
  })
})

describe('下次运行 / 耗时', () => {
  it('相对当前时间给出中文提示', () => {
    expect(formatNextRun(null, NOW)).toBe('—')
    expect(formatNextRun('2026-11-02T11:00:00Z', NOW)).toBe('等待执行')
    expect(formatNextRun('2026-11-02T12:30:00Z', NOW)).toBe('30 分钟后')
    expect(formatNextRun('2026-11-02T15:00:00Z', NOW)).toBe('3 小时后')
    expect(formatNextRun('2026-11-04T12:00:00Z', NOW)).toBe('2.0 天后')
    expect(formatNextRun('bad', NOW)).toBe('—')
  })

  it('运行耗时', () => {
    expect(formatRunDuration(null, null)).toBe('—')
    expect(formatRunDuration('2026-11-02T12:00:00Z', null)).toBe('运行中')
    expect(formatRunDuration('2026-11-02T12:00:00Z', '2026-11-02T12:00:02Z')).toBe('2s')
    expect(formatRunDuration('2026-11-02T12:00:00Z', '2026-11-02T12:01:05Z')).toBe('1m 5s')
    expect(formatRunDuration('2026-11-02T12:05:00Z', '2026-11-02T12:00:00Z')).toBe('—')
  })

  it('关联工单只接受 uuid', () => {
    expect(runTicketLink('aaaaaaaa-1111-2222-3333-444444444444')).toBe(
      'aaaaaaaa-1111-2222-3333-444444444444',
    )
    expect(runTicketLink(null)).toBeNull()
    expect(runTicketLink('not-a-uuid')).toBeNull()
  })
})

describe('巡检计划草稿校验', () => {
  const valid = {
    name: '每日机房巡检',
    scope_type: 'global',
    scope_id: '',
    interval_hours: 24,
    enabled: true,
  }

  it('合法草稿无错误', () => {
    const errors = validateInspectionDraft(valid)
    expect(hasInspectionErrors(errors)).toBe(false)
    expect(errors).toEqual({})
  })

  it('名称必填且限长', () => {
    expect(validateInspectionDraft({ ...valid, name: '  ' }).name).toContain('请输入')
    expect(validateInspectionDraft({ ...valid, name: 'x'.repeat(101) }).name).toContain('不能超过')
  })

  it('area/room 必须选择对象，global 不要求', () => {
    expect(validateInspectionDraft({ ...valid, scope_type: 'area' }).scope_id).toBe('请选择区域')
    expect(validateInspectionDraft({ ...valid, scope_type: 'room' }).scope_id).toBe('请选择房间')
    expect(validateInspectionDraft({ ...valid, scope_type: 'room', scope_id: 'x' }).scope_id).toBe(
      undefined,
    )
    expect(validateInspectionDraft(valid).scope_id).toBeUndefined()
  })

  it('间隔必须是 1–168 的整数', () => {
    expect(validateInspectionDraft({ ...valid, interval_hours: 0 }).interval_hours).toContain(
      '1–168',
    )
    expect(validateInspectionDraft({ ...valid, interval_hours: 169 }).interval_hours).toContain(
      '1–168',
    )
    expect(validateInspectionDraft({ ...valid, interval_hours: 1.5 }).interval_hours).toContain(
      '整数',
    )
    expect(validateInspectionDraft({ ...valid, interval_hours: '48' }).interval_hours).toBe(
      undefined,
    )
  })
})
