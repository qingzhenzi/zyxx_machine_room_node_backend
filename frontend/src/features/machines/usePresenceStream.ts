import { onScopeDispose, ref } from 'vue'

import { ApiError, api, streamDeviceEvents } from '@/api'
import type { PresenceEvent } from '@/api/schema'

export type PresenceStreamStatus = 'connecting' | 'live' | 'offline'

/**
 * 设备在线状态实时流（`device.presence`）：进入页面即连接，只接收连接之后的新事件，
 * 断线按 3s 退避重连；401 时先用一次普通请求触发 axios 刷新 token，再重连。
 *
 * 与 `useMachineStream` 一样，底层用 fetch + ReadableStream（EventSource 不能带 Authorization）。
 */
export function usePresenceStream(options: { onEvent: (event: PresenceEvent) => void }) {
  const status = ref<PresenceStreamStatus>('connecting')

  let controller: AbortController | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let disposed = false

  function clearRetry() {
    if (retryTimer) clearTimeout(retryTimer)
    retryTimer = null
  }

  function scheduleReconnect() {
    if (disposed) return
    clearRetry()
    status.value = 'offline'
    retryTimer = setTimeout(() => {
      retryTimer = null
      void connect()
    }, 3_000)
  }

  async function connect() {
    if (disposed) return
    clearRetry()
    status.value = 'connecting'
    const local = new AbortController()
    controller = local

    try {
      await streamDeviceEvents({
        signal: local.signal,
        onEvent: (event) => {
          status.value = 'live'
          options.onEvent(event)
        },
        onHeartbeat: () => {
          status.value = 'live'
        },
      })
      // 正常结束（服务端关闭 / 代理超时）也需要重连。
      if (!disposed && !local.signal.aborted) scheduleReconnect()
    } catch (error) {
      if (disposed || local.signal.aborted) return
      // fetch 不走 axios 拦截器：401 时先触发一次普通请求，让 axios 用 refresh token
      // 换新 access token，再重连；否则 access token 过期后 SSE 会一直 401。
      if (error instanceof ApiError && error.status === 401) {
        await api.me().catch(() => undefined)
      }
      scheduleReconnect()
    }
  }

  function stop() {
    disposed = true
    clearRetry()
    controller?.abort()
    controller = null
  }

  void connect()
  onScopeDispose(stop)

  return { status, reconnect: connect, stop }
}
