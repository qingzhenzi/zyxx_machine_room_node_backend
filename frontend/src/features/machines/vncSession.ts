//! noVNC 控制台的纯逻辑：WS 地址拼接、连接状态文案、运行状态提示。
//!
//! 后端契约（docs/novnc-design.md §5）：
//! - `POST /api/v1/admin/machines/{serial}/vnc/session` body `{vm}` →
//!   `{session_id, ticket, vm, password, ws_url?, expires_at}`；
//! - WS：`ws(s)://<control-host>/api/v1/admin/machines/{serial}/vnc/ws?session=<id>&ticket=<ticket>`
//!   （`ws_url` 缺省时使用上面这条路径）。

import type { BadgeTone } from '@/components/ui/StateBadge.vue'

import { isVmRunningState } from './vmCommand'

/** 后端缺省 WS 端点；与 `GET /admin/machines/{serial}/vnc/ws` 保持一致。 */
export function defaultVncWsPath(serial: string): string {
  return `/api/v1/admin/machines/${encodeURIComponent(serial)}/vnc/ws`
}

export interface VncWsUrlInput {
  serial: string
  sessionId: string
  ticket: string
  /** 后端返回的可选 ws_url（相对路径或绝对 ws(s)/http(s) URL）。 */
  wsUrl?: string | null
  /** 解析相对路径用的页面地址；缺省读取 `window.location.href`。 */
  pageUrl?: string
}

/**
 * 把 session/ticket 拼进 WebSocket URL：
 * - 相对路径 → 按当前页面 origin 解析；
 * - http(s) → ws(s)（同源部署常见）；
 * - 已有 query 保留，`session`/`ticket` 覆盖同名参数并做 URL 编码。
 */
export function buildVncWsUrl(input: VncWsUrlInput): string {
  const pageUrl =
    input.pageUrl ?? (typeof window === 'undefined' ? 'http://localhost/' : window.location.href)
  const raw = input.wsUrl?.trim() || defaultVncWsPath(input.serial)

  const url = new URL(raw, pageUrl)
  if (url.protocol === 'http:') {
    url.protocol = 'ws:'
  } else if (url.protocol === 'https:') {
    url.protocol = 'wss:'
  } else if (url.protocol !== 'ws:' && url.protocol !== 'wss:') {
    throw new Error(`不支持的 VNC WebSocket 协议：${url.protocol}`)
  }

  url.searchParams.set('session', input.sessionId)
  url.searchParams.set('ticket', input.ticket)
  return url.toString()
}

export type VncConnectionStatus =
  | 'idle'
  | 'requesting'
  | 'connecting'
  | 'connected'
  | 'closed'
  | 'error'

export interface VncStatusMeta {
  label: string
  tone: BadgeTone
}

const VNC_STATUS_META: Record<VncConnectionStatus, VncStatusMeta> = {
  idle: { label: '未连接', tone: 'slate' },
  requesting: { label: '申请会话中', tone: 'amber' },
  connecting: { label: '连接中', tone: 'amber' },
  connected: { label: '已连接', tone: 'emerald' },
  closed: { label: '已断开', tone: 'slate' },
  error: { label: '连接失败', tone: 'rose' },
}

/** 状态徽标文案/色调（空值按 idle 处理）。 */
export function vncConnectionMeta(status: VncConnectionStatus | null | undefined): VncStatusMeta {
  return status ? VNC_STATUS_META[status] : VNC_STATUS_META.idle
}

/** 是否处于「正在建立连接」状态（按钮 loading）。 */
export function isVncBusyStatus(status: VncConnectionStatus | null | undefined): boolean {
  return status === 'requesting' || status === 'connecting'
}

/**
 * VM 行「控制台」按钮的提示文案。
 * libvirt VNC 只在虚拟机运行时可用；关机/未运行先提示启动。
 */
export function vncRuntimeHint(state: string | null | undefined): string {
  if (isVmRunningState(state)) return '打开实时控制台（noVNC）'
  return `虚拟机当前状态为 ${(state ?? '').trim() || '未知'}，请先启动后再打开控制台`
}
