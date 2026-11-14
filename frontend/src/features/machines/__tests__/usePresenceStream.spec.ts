import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError, api, streamDeviceEvents } from '@/api'
import type { DeviceEventStreamOptions } from '@/api'
import type { PresenceEvent } from '@/api/schema'

import { usePresenceStream } from '../usePresenceStream'

vi.mock('@/api', () => {
  class MockApiError extends Error {
    readonly status?: number

    constructor(message: string, status?: number) {
      super(message)
      this.name = 'ApiError'
      this.status = status
    }
  }

  return {
    ApiError: MockApiError,
    api: { me: vi.fn<() => Promise<unknown>>(() => Promise.resolve({})) },
    streamDeviceEvents: vi.fn<(options: DeviceEventStreamOptions) => Promise<void>>(),
  }
})

const streamMock = vi.mocked(streamDeviceEvents)
const meMock = vi.mocked(api.me)

const offlineEvent: PresenceEvent = {
  id: 7,
  serial: 'SERIAL-0001',
  device_slot: 'slot-01',
  current_state: 'offline',
  previous_state: 'online',
  occurred_at: '2026-05-01T10:00:00Z',
  payload: {},
}

beforeEach(() => {
  streamMock.mockReset()
  meMock.mockClear()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('usePresenceStream', () => {
  it('forwards presence events, reports live, and aborts on scope dispose', () => {
    const calls: DeviceEventStreamOptions[] = []
    streamMock.mockImplementation((options) => {
      calls.push(options)
      return new Promise<void>(() => undefined)
    })

    const onEvent = vi.fn<(event: PresenceEvent) => void>()
    const scope = effectScope()
    const stream = scope.run(() => usePresenceStream({ onEvent }))!

    expect(stream.status.value).toBe('connecting')
    const active = calls[0]!
    active.onEvent(offlineEvent)
    expect(stream.status.value).toBe('live')
    expect(onEvent).toHaveBeenCalledWith(offlineEvent)

    scope.stop()
    expect(active.signal.aborted).toBe(true)
  })

  it('refreshes the session on 401 before reconnecting after 3s', async () => {
    vi.useFakeTimers()
    streamMock
      .mockRejectedValueOnce(new ApiError('登录已过期', 401))
      .mockImplementation(() => new Promise<void>(() => undefined))

    const scope = effectScope()
    const stream = scope.run(() =>
      usePresenceStream({ onEvent: vi.fn<(event: PresenceEvent) => void>() }),
    )!

    await vi.advanceTimersByTimeAsync(0)
    expect(meMock).toHaveBeenCalledTimes(1)
    expect(stream.status.value).toBe('offline')
    expect(streamMock).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(3_000)
    expect(streamMock).toHaveBeenCalledTimes(2)
    expect(stream.status.value).toBe('connecting')

    scope.stop()
  })

  it('reconnects 3s after the server closes the stream', async () => {
    vi.useFakeTimers()
    streamMock
      .mockResolvedValueOnce(undefined)
      .mockImplementation(() => new Promise<void>(() => undefined))

    const scope = effectScope()
    const stream = scope.run(() =>
      usePresenceStream({ onEvent: vi.fn<(event: PresenceEvent) => void>() }),
    )!

    await vi.advanceTimersByTimeAsync(0)
    expect(streamMock).toHaveBeenCalledTimes(1)
    expect(stream.status.value).toBe('offline')

    await vi.advanceTimersByTimeAsync(2_999)
    expect(streamMock).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(streamMock).toHaveBeenCalledTimes(2)

    scope.stop()
  })
})
