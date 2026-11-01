import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'

import App from '../App.vue'
import { installRouterGuards } from '../router/guards'
import { routes } from '../router/routes'

// Node 自带的实验性 localStorage 会盖住 jsdom 的实现，而且没有方法。
class MemoryStorage implements Storage {
  private readonly entries = new Map<string, string>()

  get length() {
    return this.entries.size
  }

  clear() {
    this.entries.clear()
  }

  getItem(key: string) {
    return this.entries.get(key) ?? null
  }

  key(index: number) {
    return Array.from(this.entries.keys())[index] ?? null
  }

  removeItem(key: string) {
    this.entries.delete(key)
  }

  setItem(key: string, value: string) {
    this.entries.set(key, String(value))
  }
}

const user = {
  id: '980dfb1a-da14-408b-a15a-e3b3da296731',
  username: 'admin',
  display_name: 'Administrator',
  email: null,
  is_active: true,
  roles: ['admin'],
  permissions: ['room:read', 'device:read'],
  created_at: new Date().toISOString(),
}

const session = {
  accessToken: 'test-access',
  refreshToken: 'test-refresh',
  expiresIn: 900,
  user,
}

vi.mock('@/api', () => ({
  api: {
    refresh: vi.fn<() => Promise<typeof session>>(async () => session),
    me: vi.fn<() => Promise<typeof user>>(async () => user),
    logout: vi.fn<() => Promise<void>>(async () => undefined),
    login: vi.fn<() => Promise<typeof session>>(async () => session),
    forgotPassword: vi.fn<() => Promise<void>>(async () => undefined),
  },
  setUnauthorizedHandler: vi.fn<(handler: (() => void) | null) => void>(),
}))

beforeEach(() => {
  vi.stubGlobal('localStorage', new MemoryStorage())
  vi.stubGlobal('sessionStorage', new MemoryStorage())
  Element.prototype.scrollTo = () => {}
  setActivePinia(createPinia())
})

function createTestRouter() {
  const router = createRouter({ history: createMemoryHistory(), routes })
  installRouterGuards(router)
  return router
}

describe('router guards', () => {
  it('sends anonymous visitors to the login page', async () => {
    const router = createTestRouter()
    await router.push('/rooms')
    await flushPromises()

    expect(router.currentRoute.value.name).toBe('login')
    expect(router.currentRoute.value.query.redirect).toBe('/rooms')
  })

  it('keeps an authenticated user on the requested page', async () => {
    localStorage.setItem('rn:auth:refresh', 'test-refresh')
    const router = createTestRouter()
    await router.push('/rooms')
    await flushPromises()

    expect(router.currentRoute.value.name).toBe('rooms')
  })

  it('redirects to forbidden when the user lacks the page permission', async () => {
    localStorage.setItem('rn:auth:refresh', 'test-refresh')
    const router = createTestRouter()
    await router.push('/users')
    await flushPromises()

    expect(router.currentRoute.value.name).toBe('forbidden')
  })
})

describe('login page', () => {
  it('renders the standalone login layout with product branding', async () => {
    const router = createTestRouter()
    const wrapper = mount(App, {
      global: { plugins: [createPinia(), router] },
    })

    await router.push('/login')
    await router.isReady()
    await flushPromises()

    expect(wrapper.text()).toContain('Room Node Console')
    expect(wrapper.text()).toContain('使用房间节点管理账号继续')
    expect(wrapper.find('h1').text()).toBe('登录')
  })
})
