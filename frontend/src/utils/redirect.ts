/**
 * 只允许站内跳转，挡掉 `?redirect=//evil.com` 这类开放重定向。
 */
export function safeRedirect(value: unknown, fallback = '/'): string {
  const raw = Array.isArray(value) ? value[0] : value
  if (typeof raw !== 'string') return fallback
  if (!raw.startsWith('/') || raw.startsWith('//')) return fallback
  return raw
}
