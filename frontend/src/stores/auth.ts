import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { api } from '@/api'
import {
  clearSession,
  getRefreshToken,
  isRemembered,
  setAccessToken,
  setRefreshToken,
} from '@/api/session'
import type { AuthSession, AuthUser } from '@/api/types'
import { errorMessage } from '@/utils/errors'

export type AuthStatus = 'idle' | 'restoring' | 'authenticated' | 'anonymous'

export const useAuthStore = defineStore('auth', () => {
  const user = ref<AuthUser | null>(null)
  const status = ref<AuthStatus>('idle')
  const lastError = ref<string | null>(null)

  let restorePromise: Promise<void> | null = null
  let lastProfileAt = 0
  const PROFILE_TTL_MS = 60_000

  const isAuthenticated = computed(() => status.value === 'authenticated' && user.value !== null)
  const permissions = computed(() => user.value?.permissions ?? [])
  const roles = computed(() => user.value?.roles ?? [])
  const displayName = computed(() => user.value?.display_name || user.value?.username || '未登录')

  /** 判断当前用户是否拥有某个权限点。 */
  function can(permission: string): boolean {
    return permissions.value.includes(permission)
  }

  function canAny(...codes: string[]): boolean {
    return codes.some((code) => can(code))
  }

  function apply(session: AuthSession, remember: boolean) {
    setAccessToken(session.accessToken)
    setRefreshToken(session.refreshToken, remember)
    user.value = session.user
    status.value = 'authenticated'
    lastError.value = null
  }

  /** 只清本地状态，不请求服务端。 */
  function clear() {
    clearSession()
    user.value = null
    status.value = 'anonymous'
  }

  async function login(username: string, password: string, remember: boolean) {
    status.value = 'restoring'
    lastError.value = null
    try {
      apply(await api.login({ username, password }), remember)
    } catch (error) {
      status.value = 'anonymous'
      lastError.value = errorMessage(error, '登录失败')
      throw error
    }
  }

  /**
   * 页面刷新后用 refresh token 恢复会话。并发调用共用同一个请求，
   * 因为刷新令牌是一次性的，多请求会互相失效。
   */
  async function restore() {
    if (status.value === 'authenticated') return
    if (restorePromise) return restorePromise

    const refreshToken = getRefreshToken()
    if (!refreshToken) {
      status.value = 'anonymous'
      return
    }

    restorePromise = (async () => {
      status.value = 'restoring'
      try {
        apply(await api.refresh(refreshToken), isRemembered())
      } catch {
        clear()
      } finally {
        restorePromise = null
      }
    })()

    return restorePromise
  }

  /**
   * 重新拉取当前用户资料（角色/权限可能已被管理员改动）。
   * 默认 60 秒内复用上一次结果，避免每次路由跳转都多一次网络往返。
   */
  async function refreshProfile(force = false) {
    if (!isAuthenticated.value) return
    if (!force && Date.now() - lastProfileAt < PROFILE_TTL_MS) return
    try {
      user.value = await api.me()
      lastProfileAt = Date.now()
    } catch {
      // 失败时保留旧资料，下一次导航还会再校验
    }
  }

  async function logout() {
    const refreshToken = getRefreshToken()
    try {
      if (refreshToken) await api.logout(refreshToken)
    } catch {
      // 服务端注销失败不影响本地退出
    } finally {
      clear()
    }
  }

  return {
    user,
    status,
    lastError,
    isAuthenticated,
    permissions,
    roles,
    displayName,
    can,
    canAny,
    login,
    logout,
    restore,
    refreshProfile,
    clear,
  }
})
