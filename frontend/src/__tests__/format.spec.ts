import { describe, expect, it } from 'vitest'

import {
  formatBytes,
  formatDuration,
  formatPercent,
  formatRelative,
  shortId,
  weekdayLabel,
} from '../utils/format'

describe('format helpers', () => {
  it('formats byte sizes', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1024)).toBe('1.00 KiB')
    expect(formatBytes(10 * 1024 * 1024)).toBe('10.0 MiB')
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.00 GiB')
    expect(formatBytes(null)).toBe('—')
  })

  it('formats durations and percentages', () => {
    expect(formatDuration(40)).toBe('40ms')
    expect(formatDuration(61_000)).toBe('1m 1s')
    expect(formatDuration(null)).toBe('—')
    expect(formatPercent(98.256, 1)).toBe('98.3%')
    expect(formatPercent(null)).toBe('—')
  })

  it('formats weekday labels (1 = Monday)', () => {
    expect(weekdayLabel(1)).toBe('周一')
    expect(weekdayLabel(7)).toBe('周日')
    expect(weekdayLabel(0)).toBe('—')
  })

  it('shortens ids and handles relative time', () => {
    expect(shortId('980dfb1a-da14-408b-a15a-e3b3da296731')).toBe('980dfb1a')
    expect(shortId(null)).toBe('—')
    expect(formatRelative(new Date().toISOString())).toBe('刚刚')
    expect(formatRelative(null)).toBe('—')
  })
})
