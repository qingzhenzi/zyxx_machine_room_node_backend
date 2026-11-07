import { describe, expect, it } from 'vitest'

import { parseMachinePayload } from '../payload'

describe('parseMachinePayload', () => {
  it('extracts macs from sections.agent.host.macs', () => {
    const parsed = parseMachinePayload({
      sections: {
        agent: {
          host: {
            macs: ['AA:BB:CC:DD:EE:01', 'aa:bb:cc:dd:ee:02'],
          },
        },
      },
    })
    expect(parsed.agent?.macs).toEqual(['aa:bb:cc:dd:ee:01', 'aa:bb:cc:dd:ee:02'])
  })

  it('returns empty macs for legacy agents and garbage values', () => {
    // 完全没有 sections.agent 的旧 agent：agent 视图为 null（页面显示「未采集」）。
    expect(parseMachinePayload({}).agent).toBeNull()
    // 有 agent 但 macs 缺失/非法：返回空数组，控制台据此禁用「网络唤醒」。
    const parsed = parseMachinePayload({
      sections: { agent: { host: { macs: ['not-a-mac', 42, null] } } },
    })
    expect(parsed.agent?.macs).toEqual([])
  })
})
