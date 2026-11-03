import { describe, expect, it } from 'vitest'

import {
  buildDailySeries,
  buildInsight,
  buildStatsRange,
  formatMttr,
  isOverdue,
  maxCount,
  normalizeTotals,
  percentage,
  summarizeSeries,
  toAssigneeRows,
  toCategoryRows,
  toDayKey,
} from '../ticketStats'

describe('toDayKey', () => {
  it('规范化日期与 RFC3339 时间', () => {
    expect(toDayKey('2026-11-01')).toBe('2026-11-01')
    expect(toDayKey('2026-11-01T12:30:00Z')).toBe('2026-11-01')
    expect(toDayKey('2026-11-01T00:30:00+08:00')).toBe('2026-10-31')
    expect(toDayKey(null)).toBeNull()
    expect(toDayKey('not-a-date')).toBeNull()
  })
})

describe('buildStatsRange', () => {
  it('按 UTC 日对齐，days=7 时覆盖 7 个日桶', () => {
    const now = new Date('2026-11-01T10:00:00Z')
    const { from, to } = buildStatsRange(7, now)
    expect(from).toBe('2026-10-26T00:00:00.000Z')
    expect(to).toBe('2026-11-01T10:00:00.000Z')
    const series = buildDailySeries([], from, to)
    expect(series).toHaveLength(7)
    expect(series[0]?.date).toBe('2026-10-26')
    expect(series[6]?.date).toBe('2026-11-01')
  })

  it('days 非法时按 1 天处理', () => {
    const { from, to } = buildStatsRange(0, new Date('2026-11-01T10:00:00Z'))
    expect(buildDailySeries([], from, to)).toHaveLength(1)
  })
})

describe('buildDailySeries', () => {
  const from = '2026-10-26T00:00:00.000Z'
  const to = '2026-11-01T10:00:00.000Z'

  it('缺失日期补 0，已存在日期累加', () => {
    const series = buildDailySeries(
      [
        { date: '2026-10-28', created: 2, resolved: 1, closed: 0 },
        { date: '2026-10-30', created: 1, resolved: 0, closed: 1 },
      ],
      from,
      to,
    )
    expect(series).toHaveLength(7)
    expect(series[2]).toEqual({ date: '2026-10-28', created: 2, resolved: 1, closed: 0 })
    expect(series[4]).toEqual({ date: '2026-10-30', created: 1, resolved: 0, closed: 1 })
    expect(series[0]).toEqual({ date: '2026-10-26', created: 0, resolved: 0, closed: 0 })
  })

  it('窗口外的数据点自动扩展，不丢数', () => {
    const series = buildDailySeries([{ date: '2026-10-20', created: 4 }], from, to)
    expect(series[0]?.date).toBe('2026-10-20')
    expect(series[0]?.created).toBe(4)
    expect(series[series.length - 1]?.date).toBe('2026-11-01')
  })

  it('缺少 from/to 时按数据点排序返回', () => {
    const series = buildDailySeries(
      [
        { date: '2026-10-30', created: 1 },
        { date: '2026-10-28', created: 2 },
      ],
      null,
      null,
    )
    expect(series.map((point) => point.date)).toEqual(['2026-10-28', '2026-10-29', '2026-10-30'])
  })

  it('忽略非法日期，空数据返回空序列', () => {
    expect(buildDailySeries([{ date: 'bad', created: 1 }], from, to)).toHaveLength(7)
    expect(buildDailySeries([], null, null)).toEqual([])
  })
})

describe('summarizeSeries', () => {
  it('汇总新增 / 关闭 / 净增', () => {
    const summary = summarizeSeries([
      { date: '2026-10-26', created: 3, resolved: 1, closed: 0 },
      { date: '2026-10-27', created: 2, resolved: 4, closed: 1 },
    ])
    expect(summary).toEqual({ created: 5, resolved: 5, closed: 1, net: 0 })
  })
})

describe('normalizeTotals', () => {
  it('缺失 / 非法数值回落为 0 或 null', () => {
    expect(normalizeTotals(null)).toEqual({
      created: 0,
      resolved: 0,
      closed: 0,
      open_now: 0,
      overdue: 0,
      avg_resolve_secs: null,
      median_resolve_secs: null,
    })
    expect(
      normalizeTotals({
        created: 4,
        resolved: 2,
        closed: 1,
        open_now: 3,
        overdue: 1,
        avg_resolve_secs: 3600,
        median_resolve_secs: -5,
      }),
    ).toEqual({
      created: 4,
      resolved: 2,
      closed: 1,
      open_now: 3,
      overdue: 1,
      avg_resolve_secs: 3600,
      median_resolve_secs: null,
    })
  })
})

describe('formatMttr', () => {
  it('按秒 / 分钟 / 小时 / 天输出中文时长', () => {
    expect(formatMttr(null)).toBe('—')
    expect(formatMttr(-1)).toBe('—')
    expect(formatMttr(30)).toBe('30 秒')
    expect(formatMttr(90)).toBe('2 分钟')
    expect(formatMttr(3_600)).toBe('1.0 小时')
    expect(formatMttr(90_000)).toBe('1.0 天')
  })
})

describe('percentage / maxCount', () => {
  it('百分比取整并夹在 0–100', () => {
    expect(percentage(1, 3)).toBe(33)
    expect(percentage(0, 0)).toBe(0)
    expect(percentage(5, 3)).toBe(100)
  })

  it('maxCount 至少为 1，避免除零', () => {
    expect(maxCount([])).toBe(1)
    expect(maxCount([{ count: 0 }, { count: 4 }])).toBe(4)
  })
})

describe('toCategoryRows / toAssigneeRows', () => {
  it('分类归一化、过滤 0 并倒序', () => {
    const rows = toCategoryRows([
      { category: 'other', count: 1 },
      { category: 'machine_health', count: 5 },
      { category: 'security', count: 0 },
      { category: null, count: 2 },
    ])
    expect(rows.map((row) => row.category)).toEqual(['machine_health', 'other', 'other'])
    expect(rows[0]?.count).toBe(5)
    expect(rows).toHaveLength(3)
  })

  it('处理人负载过滤空行并倒序', () => {
    const rows = toAssigneeRows([
      { assignee_id: 'a', display_name: '张伟', active: 1, resolved: 2, avg_resolve_secs: 3600 },
      { assignee_id: 'b', display_name: '李娜', active: 4, resolved: 3 },
      { assignee_id: 'c', display_name: '王强', active: 0, resolved: 0 },
    ])
    expect(rows.map((row) => row.display_name)).toEqual(['李娜', '张伟'])
    expect(rows[1]?.avg_resolve_secs).toBe(3600)
  })
})

describe('isOverdue', () => {
  const now = Date.parse('2026-11-01T10:00:00Z')

  it('只有未关闭且已过 SLA 才算超时', () => {
    expect(isOverdue({ status: 'open', due_at: '2026-11-01T09:00:00Z' }, now)).toBe(true)
    expect(isOverdue({ status: 'in_progress', due_at: '2026-11-01T09:00:00Z' }, now)).toBe(true)
    expect(isOverdue({ status: 'resolved', due_at: '2026-11-01T09:00:00Z' }, now)).toBe(false)
    expect(isOverdue({ status: 'open', due_at: '2026-11-01T11:00:00Z' }, now)).toBe(false)
    expect(isOverdue({ status: 'open', due_at: null }, now)).toBe(false)
    expect(isOverdue({ status: 'open', due_at: 'bad' }, now)).toBe(false)
  })
})

describe('buildInsight', () => {
  const totals = normalizeTotals({ created: 10, resolved: 6, open_now: 4, overdue: 1 })

  it('无新增时给出中性结论', () => {
    const insight = buildInsight({
      days: 7,
      totals: normalizeTotals(null),
      byCategory: [],
      topRooms: [],
    })
    expect(insight.text).toContain('暂无新增工单')
  })

  it('存在同房间重复故障时给出重复故障提醒', () => {
    const insight = buildInsight({
      days: 30,
      totals,
      byCategory: [
        { category: 'machine_health', count: 6 },
        { category: 'other', count: 4 },
      ],
      topRooms: [{ room_id: 'r1', room_name: '智慧教室 A', count: 5 }],
    })
    expect(insight.title).toBe('重复故障提醒')
    expect(insight.text).toContain('智慧教室 A')
    expect(insight.text).toContain('机器健康类最多')
    expect(insight.text).toContain('60%')
  })

  it('分布分散时给出观察结论', () => {
    const insight = buildInsight({
      days: 7,
      totals,
      byCategory: [{ category: 'device_offline', count: 3 }],
      topRooms: [{ room_id: 'r1', room_name: '计算机房 B', count: 1 }],
    })
    expect(insight.title).toBe('自动洞察')
    expect(insight.text).toContain('计算机房 B')
    expect(insight.text).toContain('分散')
  })
})
