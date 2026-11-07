import { describe, expect, it } from 'vitest'

import {
  buildVncWsUrl,
  defaultVncWsPath,
  isVncBusyStatus,
  vncConnectionMeta,
  vncRuntimeHint,
} from '../vncSession'

const PAGE_URL = 'https://console.example.com/machines/SN-01'

describe('buildVncWsUrl', () => {
  it('缺省端点：相对路径按页面 origin 解析并转成 ws(s)', () => {
    const url = buildVncWsUrl({
      serial: 'SN-01',
      sessionId: 'sess-1',
      ticket: 'ticket-1',
      pageUrl: PAGE_URL,
    })
    expect(url).toBe(
      'wss://console.example.com/api/v1/admin/machines/SN-01/vnc/ws?session=sess-1&ticket=ticket-1',
    )
  })

  it('http 页面 → ws（本地开发/反向代理）', () => {
    const url = buildVncWsUrl({
      serial: 'SN-01',
      sessionId: 's',
      ticket: 't',
      pageUrl: 'http://127.0.0.1:5174/machines/SN-01',
    })
    expect(url.startsWith('ws://127.0.0.1:5174/api/v1/')).toBe(true)
  })

  it('serial/ticket 做 URL 转义（防注入/特殊字符）', () => {
    const url = buildVncWsUrl({
      serial: 'SN 01/a&b',
      sessionId: 'sess 1',
      ticket: 'a+b/c==&d',
      pageUrl: PAGE_URL,
    })
    expect(url).toContain('/machines/SN%2001%2Fa%26b/vnc/ws')
    expect(url).toContain('session=sess+1')
    expect(url).toContain('ticket=a%2Bb%2Fc%3D%3D%26d')
  })

  it('后端返回 ws_url 时保留其 host/path/query（绝对 wss）', () => {
    const url = buildVncWsUrl({
      serial: 'SN-01',
      sessionId: 'sess',
      ticket: 'tk',
      wsUrl: 'wss://control.example.com/custom/vnc/ws?foo=bar',
      pageUrl: PAGE_URL,
    })
    expect(url).toBe('wss://control.example.com/custom/vnc/ws?foo=bar&session=sess&ticket=tk')
  })

  it('后端返回 http(s)/相对 ws_url 时统一转成 ws(s) 并覆盖同名参数', () => {
    expect(
      buildVncWsUrl({
        serial: 'SN-01',
        sessionId: 'new-sess',
        ticket: 'new-ticket',
        wsUrl: 'https://control.example.com/api/v1/admin/machines/SN-01/vnc/ws?session=old',
        pageUrl: PAGE_URL,
      }),
    ).toBe(
      'wss://control.example.com/api/v1/admin/machines/SN-01/vnc/ws?session=new-sess&ticket=new-ticket',
    )

    expect(
      buildVncWsUrl({
        serial: 'SN-01',
        sessionId: 's',
        ticket: 't',
        wsUrl: '/api/v1/admin/machines/SN-01/vnc/ws',
        pageUrl: PAGE_URL,
      }),
    ).toBe('wss://console.example.com/api/v1/admin/machines/SN-01/vnc/ws?session=s&ticket=t')
  })

  it('拒绝非 ws(s)/http(s) 协议', () => {
    expect(() =>
      buildVncWsUrl({
        serial: 'SN-01',
        sessionId: 's',
        ticket: 't',
        wsUrl: 'ftp://control.example.com/vnc',
        pageUrl: PAGE_URL,
      }),
    ).toThrow(/不支持的 VNC WebSocket 协议/)
  })

  it('defaultVncWsPath 对 serial 编码', () => {
    expect(defaultVncWsPath('SN 01/a')).toBe('/api/v1/admin/machines/SN%2001%2Fa/vnc/ws')
  })
})

describe('连接状态纯函数', () => {
  it('状态徽标文案/色调', () => {
    expect(vncConnectionMeta('connected')).toEqual({ label: '已连接', tone: 'emerald' })
    expect(vncConnectionMeta('requesting')).toEqual({ label: '申请会话中', tone: 'amber' })
    expect(vncConnectionMeta('connecting')).toEqual({ label: '连接中', tone: 'amber' })
    expect(vncConnectionMeta('error')).toEqual({ label: '连接失败', tone: 'rose' })
    expect(vncConnectionMeta('closed')).toEqual({ label: '已断开', tone: 'slate' })
    expect(vncConnectionMeta(null)).toEqual({ label: '未连接', tone: 'slate' })
  })

  it('isVncBusyStatus 只覆盖申请/连接中', () => {
    expect(isVncBusyStatus('requesting')).toBe(true)
    expect(isVncBusyStatus('connecting')).toBe(true)
    expect(isVncBusyStatus('connected')).toBe(false)
    expect(isVncBusyStatus('idle')).toBe(false)
    expect(isVncBusyStatus(null)).toBe(false)
  })

  it('vncRuntimeHint：运行中才提示可打开，否则提示先启动', () => {
    expect(vncRuntimeHint('running')).toBe('打开实时控制台（noVNC）')
    expect(vncRuntimeHint('paused')).toBe('打开实时控制台（noVNC）')
    expect(vncRuntimeHint('shut off')).toContain('请先启动')
    expect(vncRuntimeHint(null)).toContain('未知')
    expect(vncRuntimeHint('shut off')).toContain('shut off')
  })
})
