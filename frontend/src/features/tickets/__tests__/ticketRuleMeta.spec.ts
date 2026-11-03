import { describe, expect, it } from 'vitest'

import type { TicketStreamEvent } from '@/api/schema'
import {
  buildThresholdPayload,
  defaultThresholds,
  formatRuleScope,
  formatRuleThresholds,
  hasRuleErrors,
  normalizeSignalType,
  validateRuleDraft,
} from '@/features/tickets/ticketRuleMeta'
import {
  eventMatchesTicket,
  nextEventCursor,
  ticketStreamStatusMeta,
} from '@/features/tickets/useTicketStream'

describe('ticketRuleMeta', () => {
  it('recognizes the six signal types and defaults thresholds', () => {
    expect(normalizeSignalType('machine_health')).toBe('machine_health')
    expect(normalizeSignalType('unknown')).toBeNull()
    expect(defaultThresholds('machine_health')).toEqual({ consecutive: 2 })
    expect(defaultThresholds('device_offline')).toEqual({ offline_minutes: 10 })
    expect(defaultThresholds('transfer')).toEqual({ lookback_hours: 24 })
  })

  it('normalizes and formats thresholds defensively', () => {
    expect(buildThresholdPayload('machine_health', { consecutive: '4' })).toEqual({
      consecutive: 4,
    })
    // 未知键被丢弃，缺键回退默认值。
    expect(buildThresholdPayload('device_offline', { offline_minutes: 15, foo: 1 })).toEqual({
      offline_minutes: 15,
    })
    expect(formatRuleThresholds('machine_health', { consecutive: 3 })).toContain('3')
  })

  it('formats scopes with names and falls back to short ids', () => {
    expect(formatRuleScope('global', null)).toBe('全局')
    expect(
      formatRuleScope('area', '11111111-2222-3333-4444-555555555555', {
        areas: { '11111111-2222-3333-4444-555555555555': '教学区' },
      }),
    ).toBe('区域 · 教学区')
    expect(formatRuleScope('room', 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee')).toContain('房间')
  })

  it('validates drafts and falls back to defaults for missing thresholds', () => {
    const errors = validateRuleDraft({
      name: '',
      signal_type: 'machine_health',
      priority: 'high',
      scope_type: 'room',
      scope_id: '',
      enabled: true,
      thresholds: { consecutive: 99 },
    })
    expect(errors.name).toBeTruthy()
    expect(errors.scope_id).toBeTruthy()
    expect(errors.thresholds?.consecutive).toBeTruthy()
    expect(hasRuleErrors(errors)).toBe(true)
  })

  it('provides SSE cursor and ticket filtering helpers', () => {
    const event = { id: 12, ticket_id: 't1' } as TicketStreamEvent
    expect(eventMatchesTicket(event, 't1')).toBe(true)
    expect(eventMatchesTicket(event, 't2')).toBe(false)
    expect(nextEventCursor(null, event)).toBe(12)
    expect(nextEventCursor(20, event)).toBe(20)
    expect(ticketStreamStatusMeta('live').label).toBe('实时')
    expect(ticketStreamStatusMeta('offline').tone).toBe('rose')
  })
})
