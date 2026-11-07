import type { Router } from 'vue-router'

import { setUnauthorizedHandler } from '@/api'
import { finishPageProgress, startPageProgress } from '@/composables/usePageProgress'
import { useAuthStore } from '@/stores/auth'
import { safeRedirect } from '@/utils/redirect'

/**
 * 认证守卫、权限守卫、进度条与标题。抽成函数是为了让测试能复用同一套逻辑。
 */
export function installRouterGuards(router: Router) {
  router.beforeEach(async (to, from) => {
    if (from.matched.length === 0 || to.path !== from.path) startPageProgress()

    const auth = useAuthStore()

    if (to.meta.public) {
      // 已登录还去登录页，直接送回目标页。
      // 刷新页面后 store 还是 idle，所以要先尝试恢复一次会话。
      if (to.name === 'login') {
        if (!auth.isAuthenticated) await auth.restore()
        if (auth.isAuthenticated) return safeRedirect(to.query.redirect)
      }
      return true
    }

    if (!auth.isAuthenticated) await auth.restore()
    if (!auth.isAuthenticated) {
      return {
        name: 'login',
        query: to.fullPath === '/' ? {} : { redirect: to.fullPath },
      }
    }

    // 角色/权限可能在后台被修改，进入受保护页面时重新拉一次资料。
    await auth.refreshProfile()

    const required = to.meta.permission as string | undefined
    if (required && !auth.can(required)) {
      return { name: 'forbidden', query: { from: to.fullPath } }
    }
    // 支持“任一权限即可”的页面（如自动规则：ticket:rule:manage 或 ticket:manage）。
    const requiredAny = to.meta.permissionAny as string[] | undefined
    if (requiredAny?.length && !auth.canAny(...requiredAny)) {
      return { name: 'forbidden', query: { from: to.fullPath } }
    }

    // 仅系统管理员可见的页面（如总控联邦）；API 侧仍是 device:read。
    const adminOnly = to.meta.adminOnly as boolean | undefined
    if (adminOnly && !auth.roles.includes('admin')) {
      return { name: 'forbidden', query: { from: to.fullPath } }
    }

    return true
  })

  // 刷新令牌也失效时，拦截器会走到这里：清掉本地会话并回登录页。
  setUnauthorizedHandler(() => {
    const auth = useAuthStore()
    auth.clear()

    const current = router.currentRoute.value
    if (current.name === 'login') return
    void router.replace({
      name: 'login',
      query: { redirect: current.fullPath },
    })
  })

  router.afterEach((to) => {
    finishPageProgress()
    const title = to.meta.title as string | undefined
    document.title = title ? `${title} · Room Node Console` : 'Room Node Console'
  })

  router.onError(() => {
    finishPageProgress()
  })
}
