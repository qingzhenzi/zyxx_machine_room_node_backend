import { describe, expect, it } from 'vitest'

import { parseMachinePayload, parseVmList } from '../payload'

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

describe('parseVmList / vms', () => {
  it('parses sections.vms from telemetry payload', () => {
    const parsed = parseMachinePayload({
      sections: {
        vms: [
          { name: 'debian12', state: 'running', vcpus: 2, memory_bytes: 2147483648 },
          { name: 'win11', state: 'shut off', vcpus: 4, memory_bytes: 8589934592 },
        ],
      },
    })
    expect(parsed.vms).toEqual([
      { name: 'debian12', state: 'running', vcpus: 2, memoryBytes: 2147483648 },
      { name: 'win11', state: 'shut off', vcpus: 4, memoryBytes: 8589934592 },
    ])
  })

  it('parses vm.command list results ({vms:[...]})', () => {
    const vms = parseVmList({
      vms: [{ name: 'debian12', state: 'running', vcpus: 2, memory_bytes: 1024 }],
    })
    expect(vms).toEqual([{ name: 'debian12', state: 'running', vcpus: 2, memoryBytes: 1024 }])
    expect(parseVmList({})).toBeNull()
    expect(parseVmList(null)).toBeNull()
  })
})
