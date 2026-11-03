import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import type { Ticket, TicketContext, TicketEvent } from '@/api/schema'

import TicketAssigneeLoad from '../TicketAssigneeLoad.vue'
import TicketCategoryBars from '../TicketCategoryBars.vue'
import TicketContextCard from '../TicketContextCard.vue'
import TicketRankList from '../TicketRankList.vue'
import TicketStatsCards from '../TicketStatsCards.vue'
import TicketTable from '../TicketTable.vue'
import TicketTimeline from '../TicketTimeline.vue'
import TicketTrendChart from '../TicketTrendChart.vue'
import { buildDailySeries, normalizeTotals } from '../ticketStats'

const NOW = '2026-11-01T10:00:00Z'

function makeTicket(overrides: Partial<Ticket> = {}): Ticket {
  return {
    id: '11111111-1111-1111-1111-111111111111',
    code: 'T-20261101-0001',
    title: 'room-101 GPU 温度过高',
    description: 'GPU 温度持续超过阈值。',
    status: 'open',
    priority: 'urgent',
    category: 'machine_health',
    source: 'auto',
    source_type: 'machine_snapshot',
    source_id: '88',
    dedupe_key: 'machine_health:room-101',
    area_id: '22222222-2222-2222-2222-222222222222',
    room_id: '33333333-3333-3333-3333-333333333333',
    serial: '4f40ccef2a7b3d21',
    device_slot: '中控主机',
    reporter_id: null,
    reporter_name: null,
    assignee_id: null,
    assignee_name: null,
    due_at: '2026-11-01T11:00:00Z',
    acknowledged_at: null,
    resolved_at: null,
    closed_at: null,
    resolution: null,
    root_cause: null,
    occurrence_count: 2,
    last_seen_at: NOW,
    metadata: { health_status: 'critical' },
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  }
}

function makeEvent(overrides: Partial<TicketEvent> = {}): TicketEvent {
  return {
    id: 1,
    ticket_id: '11111111-1111-1111-1111-111111111111',
    kind: 'created',
    actor_id: null,
    actor_name: '系统',
    from_status: null,
    to_status: null,
    note: '',
    payload: {},
    source_type: null,
    source_id: null,
    created_at: NOW,
    ...overrides,
  }
}

const context: TicketContext = {
  room_id: '33333333-3333-3333-3333-333333333333',
  room_name: '智慧教室 A',
  room_code: 'room-101',
  area_id: '22222222-2222-2222-2222-222222222222',
  area_name: '教学楼 A · 3F',
  serial: '4f40ccef2a7b3d21',
  device_slot: '中控主机',
  health_status: 'critical',
  health_score: 42,
  collected_at: '2026-11-01T09:55:00Z',
  online: true,
}

describe('TicketTable', () => {
  it('渲染编号 / 标题 / 状态 / 优先级 / 对象 / 处理人', () => {
    const wrapper = mount(TicketTable, {
      props: {
        tickets: [
          makeTicket(),
          makeTicket({ id: 'x', code: 'T-20261101-0002', status: 'closed', assignee_name: '张伟' }),
        ],
        roomNames: { '33333333-3333-3333-3333-333333333333': '智慧教室 A' },
      },
    })
    const text = wrapper.text()
    expect(text).toContain('T-20261101-0001')
    expect(text).toContain('GPU 温度过高')
    expect(text).toContain('待处理')
    expect(text).toContain('紧急')
    expect(text).toContain('机器健康')
    expect(text).toContain('智慧教室 A')
    expect(text).toContain('×2')
    expect(text).toContain('未分配')
    expect(text).toContain('张伟')
    expect(wrapper.findAll('tbody tr')).toHaveLength(2)
  })

  it('行点击向外抛 open 事件', async () => {
    const ticket = makeTicket()
    const wrapper = mount(TicketTable, { props: { tickets: [ticket] } })
    await wrapper.find('tbody tr').trigger('click')
    expect(wrapper.emitted('open')?.[0]).toEqual([ticket])
  })

  it('空列表且 loading 时渲染骨架行', () => {
    const wrapper = mount(TicketTable, { props: { tickets: [], loading: true } })
    expect(wrapper.findAll('.animate-pulse').length).toBeGreaterThan(0)
  })
})

describe('TicketTimeline', () => {
  it('区分系统条目与人工评论', () => {
    const wrapper = mount(TicketTimeline, {
      props: {
        events: [
          makeEvent({ id: 1, kind: 'created' }),
          makeEvent({
            id: 2,
            kind: 'status_changed',
            actor_id: 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
            actor_name: '张伟',
            from_status: 'open',
            to_status: 'in_progress',
          }),
          makeEvent({
            id: 3,
            kind: 'commented',
            actor_id: 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
            actor_name: '李娜',
            note: '已重启 room-agent，温度回落。',
          }),
        ],
      },
    })
    const text = wrapper.text()
    expect(text).toContain('创建了工单')
    expect(text).toContain('状态 待处理 → 处理中')
    expect(text).toContain('已重启 room-agent，温度回落。')
    expect(text).toContain('系统')
  })

  it('无事件时展示空提示', () => {
    const wrapper = mount(TicketTimeline, { props: { events: [] } })
    expect(wrapper.text()).toContain('暂无处理记录')
  })
})

describe('TicketContextCard', () => {
  it('渲染房间 / 机器健康 / 在线状态与跳转入口', () => {
    const wrapper = mount(TicketContextCard, {
      props: { ticket: makeTicket(), context },
      global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    })
    const text = wrapper.text()
    expect(text).toContain('智慧教室 A')
    expect(text).toContain('教学楼 A · 3F')
    expect(text).toContain('4f40ccef2a7b3d21')
    expect(text).toContain('严重')
    expect(text).toContain('在线')
    expect(text).toContain('42 分')
    expect(wrapper.findAll('a').length).toBe(3)
  })

  it('无关联对象时给出人工上报说明', () => {
    const wrapper = mount(TicketContextCard, {
      props: {
        ticket: makeTicket({ room_id: null, serial: null, metadata: {} }),
        context: null,
      },
      global: { stubs: { RouterLink: true } },
    })
    expect(wrapper.text()).toContain('纯人工上报')
  })
})

describe('TicketStatsCards / TicketTrendChart', () => {
  const totals = normalizeTotals({
    created: 10,
    resolved: 6,
    closed: 2,
    open_now: 4,
    overdue: 1,
    avg_resolve_secs: 3_600,
    median_resolve_secs: 1_800,
  })

  it('KPI 卡展示五项指标与 MTTR 文案', () => {
    const wrapper = mount(TicketStatsCards, { props: { totals, days: 30 } })
    const text = wrapper.text()
    expect(text).toContain('新增工单')
    expect(text).toContain('已解决')
    expect(text).toContain('未关闭')
    expect(text).toContain('平均处理时长')
    expect(text).toContain('1.0 小时')
    expect(text).toContain('30 分钟')
    expect(text).toContain('超时工单')
  })

  it('趋势图按日补零并显示净增汇总', () => {
    const series = buildDailySeries(
      [{ date: '2026-10-30', created: 2, resolved: 1 }],
      '2026-10-29T00:00:00Z',
      '2026-10-31T00:00:00Z',
    )
    const wrapper = mount(TicketTrendChart, { props: { series } })
    const text = wrapper.text()
    expect(text).toContain('新增')
    expect(text).toContain('净增')
    expect(
      wrapper.findAll('[title]').some((node) => node.attributes('title')?.includes('2026-10-30')),
    ).toBe(true)
  })

  it('无数据时展示空态', () => {
    const series = buildDailySeries([], '2026-10-29T00:00:00Z', '2026-10-31T00:00:00Z')
    const wrapper = mount(TicketTrendChart, { props: { series } })
    expect(wrapper.text()).toContain('所选区间暂无工单数据')
  })
})

describe('分类 / 排行 / 处理人负载', () => {
  it('分类横条显示分类名与占比', () => {
    const wrapper = mount(TicketCategoryBars, {
      props: {
        items: [
          { category: 'machine_health', count: 6 },
          { category: 'device_offline', count: 4 },
        ],
      },
    })
    const text = wrapper.text()
    expect(text).toContain('机器健康')
    expect(text).toContain('设备离线')
    expect(text).toContain('60%')
  })

  it('TOP 排行显示名次与数量', () => {
    const wrapper = mount(TicketRankList, {
      props: {
        items: [
          { name: '智慧教室 A', sub: 'room-101', count: 5 },
          { name: '计算机房 B', sub: 'room-102', count: 3 },
        ],
      },
    })
    const text = wrapper.text()
    expect(text).toContain('智慧教室 A')
    expect(text).toContain('TOP 2')
    expect(text).toContain('最高 5 张')
  })

  it('处理人负载显示未关闭 / 已解决与平均时长', () => {
    const wrapper = mount(TicketAssigneeLoad, {
      props: {
        items: [
          {
            assignee_id: 'a',
            display_name: '张伟',
            active: 2,
            resolved: 3,
            avg_resolve_secs: 3_600,
          },
        ],
      },
    })
    const text = wrapper.text()
    expect(text).toContain('张伟')
    expect(text).toContain('未关闭 2')
    expect(text).toContain('已解决 3')
    expect(text).toContain('1.0 小时')
  })
})
