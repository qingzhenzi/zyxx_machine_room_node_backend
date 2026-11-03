import { onScopeDispose, ref } from 'vue'

import { ApiError, api, streamTicketEvents } from '@/api'
import type { TicketStreamEvent } from '@/api/schema'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'

export type TicketStreamStatus = 'connecting' | 'live' | 'offline'

export interface TicketStreamStatusMeta {
  label: string
  tone: BadgeTone
}

/** 列表页右上角「实时」状态点文案/色调。 */
export function ticketStreamStatusMeta(status: TicketStreamStatus): TicketStreamStatusMeta {
  switch (status) {
    case 'live':
      return { label: '实时', tone: 'emerald' }
    case 'connecting':
      return { label: '实时连接中', tone: 'amber' }
    default:
      return { label: '实时重连中', tone: 'rose' }
  }
}

/** 详情页只关心当前工单的事件。 */
export function eventMatchesTicket(
  event: Pick<TicketStreamEvent, 'ticket_id'>,
  ticketId: string,
): boolean {
  return Boolean(ticketId) && event.ticket_id === ticketId
}

/** SSE 断线补发游标：始终取已收到的最大事件 id。 */
export function nextEventCursor(
  previous: number | null,
  event: Pick<TicketStreamEvent, 'id'>,
): number {
  if (typeof event.id !== 'number' || !Number.isFinite(event.id)) return previous ?? 0
  return previous === null ? event.id : Math.max(previous, event.id)
}

/**
 * 工单事件实时流（契约 §5）：
 * - 命名导出 streamTicketEvents（fetch 流式读取），断线 3s 重连，并用最大 event.id 作为
 *   after_event_id 补发断线期间的事件；
 * - 收到事件后按 400ms 防抖回调页面，避免批量事件触发大量列表/详情请求；
 * - matches 可在防抖前过滤（详情页只响应本工单事件）。
 */
export function useTicketStream(options: {
  onEvent: (event: TicketStreamEvent) => void
  matches?: (event: TicketStreamEvent) => boolean
  debounceMs?: number
}) {
  const status = ref<TicketStreamStatus>('connecting')
  const lastEventAt = ref<string | null>(null)

  let controller: AbortController | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let lastId: number | null = null
  let disposed = false

  const { run: debouncedNotify, cancel: cancelNotify } = useDebounceFn(
    (event: TicketStreamEvent) => options.onEvent(event),
    options.debounceMs ?? 400,
  )

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
      await streamTicketEvents({
        afterEventId: lastId,
        signal: local.signal,
        onOpen: () => {
          status.value = 'live'
        },
        onEvent: (event) => {
          lastId = nextEventCursor(lastId, event)
          lastEventAt.value = new Date().toISOString()
          status.value = 'live'
          if (!options.matches || options.matches(event)) debouncedNotify(event)
        },
        onHeartbeat: () => {
          status.value = 'live'
        },
      })
      // 服务端正常关闭 / 代理超时也重连。
      if (!disposed && !local.signal.aborted) scheduleReconnect()
    } catch (error) {
      if (disposed || local.signal.aborted) return
      // fetch 不经过 axios 拦截器：401 时先触发一次普通请求换新 token 再重连。
      if (error instanceof ApiError && error.status === 401) {
        await api.me().catch(() => undefined)
      }
      scheduleReconnect()
    }
  }

  function stop() {
    disposed = true
    clearRetry()
    cancelNotify()
    controller?.abort()
    controller = null
  }

  void connect()
  onScopeDispose(stop)

  return { status, lastEventAt, reconnect: connect, stop }
}
