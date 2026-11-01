import { onScopeDispose, ref, toValue, type MaybeRefOrGetter } from 'vue'

import { ApiError, api, streamMachineSnapshots } from '@/api'
import type { MachineSnapshotPoint } from '@/api/schema'

export type MachineStreamStatus = 'connecting' | 'live' | 'offline'

/**
 * 机器快照实时流：进入页面即连接，断线按 3s 退避重连，并用最后收到的快照 id
 * 作为 after_id 补发断线期间最多 200 条。列表/详情都复用它。
 *
 * 注意：浏览器 EventSource 不能设置 Authorization，底层用 fetch + ReadableStream。
 */
export function useMachineStream(options: {
  serial?: MaybeRefOrGetter<string | undefined>
  onSnapshot: (snapshot: MachineSnapshotPoint) => void
}) {
  const status = ref<MachineStreamStatus>('connecting')
  const lastEventAt = ref<string | null>(null)

  let controller: AbortController | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let lastId: number | null = null
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
      await streamMachineSnapshots({
        afterId: lastId,
        serial: toValue(options.serial) || undefined,
        signal: local.signal,
        onOpen: () => {
          status.value = 'live'
        },
        onSnapshot: (snapshot) => {
          lastId = snapshot.id
          lastEventAt.value = new Date().toISOString()
          status.value = 'live'
          options.onSnapshot(snapshot)
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

  return { status, lastEventAt, reconnect: connect, stop }
}
