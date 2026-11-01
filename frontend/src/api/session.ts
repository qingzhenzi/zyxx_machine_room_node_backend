/**
 * 认证令牌的存放位置。
 *
 * - access token 只放内存：刷新页面即丢失，由 refresh token 换新的。
 * - refresh token 按「记住这台设备」放 localStorage，否则放 sessionStorage。
 *
 * 注意：这里不是 httpOnly cookie，XSS 仍可能读到 refresh token。
 * 彻底解决需要后端改 cookie + CSRF 防护。
 */

const REFRESH_KEY = 'rn:auth:refresh'
const REMEMBER_KEY = 'rn:auth:remember'

let accessToken: string | null = null

function read(storage: Storage, key: string): string | null {
  try {
    return storage.getItem(key)
  } catch {
    return null
  }
}

function remove(storage: Storage, key: string) {
  try {
    storage.removeItem(key)
  } catch {
    // 隐私模式下存储可能不可写，忽略即可
  }
}

export function getAccessToken(): string | null {
  return accessToken
}

export function setAccessToken(token: string | null) {
  accessToken = token
}

export function isRemembered(): boolean {
  try {
    return localStorage.getItem(REMEMBER_KEY) === '1'
  } catch {
    return false
  }
}

export function getRefreshToken(): string | null {
  return read(localStorage, REFRESH_KEY) ?? read(sessionStorage, REFRESH_KEY)
}

export function setRefreshToken(token: string, remember: boolean) {
  remove(localStorage, REFRESH_KEY)
  remove(sessionStorage, REFRESH_KEY)
  try {
    const storage = remember ? localStorage : sessionStorage
    storage.setItem(REFRESH_KEY, token)
    if (remember) localStorage.setItem(REMEMBER_KEY, '1')
    else localStorage.removeItem(REMEMBER_KEY)
  } catch {
    // 存储不可写时退化为仅本次会话有效
  }
}

export function hasRefreshToken(): boolean {
  return Boolean(getRefreshToken())
}

export function clearSession() {
  accessToken = null
  remove(localStorage, REFRESH_KEY)
  remove(sessionStorage, REFRESH_KEY)
  remove(localStorage, REMEMBER_KEY)
}
