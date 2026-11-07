import { expect, test, type Page } from '@playwright/test'

/**
 * 总控联邦 e2e（最小）：登录 → /federation 可见 → 添加节点弹窗字段可用。
 * 依赖本地后端（127.0.0.1:8080）与 admin 账号；后端无联邦路由时整组跳过。
 */

const username = process.env.E2E_USERNAME || 'admin'
const password = process.env.E2E_PASSWORD || 'Admin@12345'

let backendReady = false

async function signIn(page: Page) {
  await page.goto('/login')
  await page.getByLabel('账号').fill(username)
  await page.getByLabel('密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '登录', exact: true }).click()
  await expect(page.getByRole('heading', { name: '总览' })).toBeVisible()
}

test.beforeAll(async ({ request }) => {
  try {
    // 未登录访问联邦接口：路由存在时返回 401/403，旧后端没有该路由时返回 404。
    const response = await request.get('/api/v1/admin/federation/overview')
    backendReady = response.status() !== 404
  } catch {
    backendReady = false
  }
})

test.beforeEach(() => {
  test.skip(!backendReady, '后端未启动或没有联邦路由，跳过总控联邦 e2e')
})

test('renders federation overview and add-node dialog', async ({ page }) => {
  await signIn(page)

  await page.getByRole('link', { name: '总控联邦' }).click()
  await expect(page).toHaveURL(/\/federation$/)
  await expect(page.getByRole('heading', { name: '总控联邦', exact: true })).toBeVisible()

  // 聚合 KPI 区域始终渲染（即使没有任何联邦节点）。
  await expect(page.getByLabel('联邦聚合 KPI')).toBeVisible()

  await page.getByRole('button', { name: '添加节点' }).first().click()
  await expect(page.getByRole('heading', { name: '添加联邦节点' })).toBeVisible()
  await expect(page.getByLabel('节点名称')).toBeVisible()
  await expect(page.getByLabel('节点地址（http/https，不带路径）')).toBeVisible()
  await expect(page.getByLabel('联邦 token（远端节点的 FEDERATION_TOKEN）')).toBeVisible()

  // 关闭弹窗，不创建真实节点。
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('heading', { name: '添加联邦节点' })).toBeHidden()
})
