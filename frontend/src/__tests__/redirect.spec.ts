import { describe, expect, it } from 'vitest'

import { safeRedirect } from '../utils/redirect'

describe('safeRedirect', () => {
  it('keeps in-app paths', () => {
    expect(safeRedirect('/projects')).toBe('/projects')
    expect(safeRedirect('/runs?page=2')).toBe('/runs?page=2')
  })

  it('falls back on absolute and protocol-relative urls', () => {
    expect(safeRedirect('https://evil.example')).toBe('/')
    expect(safeRedirect('//evil.example/login')).toBe('/')
    expect(safeRedirect('javascript:alert(1)')).toBe('/')
    expect(safeRedirect(undefined)).toBe('/')
  })

  it('uses the first value when the query is repeated', () => {
    expect(safeRedirect(['/reports', '/runs'])).toBe('/reports')
  })
})
