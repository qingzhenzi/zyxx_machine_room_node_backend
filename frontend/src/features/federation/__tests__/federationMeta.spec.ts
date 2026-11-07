import { describe, expect, it } from 'vitest'

import { formatCount, nodeStatusMeta, summaryNumbers, validateNodeBaseUrl } from '../federationMeta'

describe('federationMeta', () => {
  it('节点状态徽标', () => {
    expect(nodeStatusMeta('online')).toEqual({ label: '在线', tone: 'emerald' })
    expect(nodeStatusMeta('offline')).toEqual({ label: '离线', tone: 'rose' })
    expect(nodeStatusMeta('unknown')).toEqual({ label: '未刷新', tone: 'slate' })
    expect(nodeStatusMeta(null)).toEqual({ label: '未刷新', tone: 'slate' })
  })

  it('大数字缩写', () => {
    expect(formatCount(0)).toBe('0')
    expect(formatCount(999)).toBe('999')
    expect(formatCount(1234)).toBe('1.2k')
    expect(formatCount(2_400_000)).toBe('2.4m')
    expect(formatCount(null)).toBe('—')
  })

  it('节点摘要数字', () => {
    expect(
      summaryNumbers({ rooms: 10, machines: 20, online_machines: 18, open_tickets: 3 }),
    ).toEqual([
      { label: '房间', value: 10 },
      { label: '机器', value: 20 },
      { label: '在线机器', value: 18 },
      { label: '未结束工单', value: 3 },
    ])
    expect(summaryNumbers(null).map((item) => item.value)).toEqual([0, 0, 0, 0])
  })

  it('节点地址前端校验', () => {
    expect(validateNodeBaseUrl('http://127.0.0.1:8081')).toBeNull()
    expect(validateNodeBaseUrl('https://node.example.com/')).toBeNull()
    expect(validateNodeBaseUrl('')).toMatch(/请填写/)
    expect(validateNodeBaseUrl('ftp://node')).toMatch(/http/)
    expect(validateNodeBaseUrl('http://node/path')).toMatch(/路径/)
    expect(validateNodeBaseUrl('http://node?x=1')).toMatch(/query/)
  })
})
