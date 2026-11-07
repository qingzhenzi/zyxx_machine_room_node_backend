import { expect, test, type Page } from '@playwright/test'

/**
 * 机器详情的「虚拟机」面板（VM 远程控制阶段1）e2e：
 * 登录 → /machines → 打开第一台机器 → 断言虚拟机面板与操作区渲染。
 * 依赖本地后端；后端不可用时跳过。
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
    const response = await request.get('/api/v1/admin/machines')
    backendReady = response.status() !== 404
  } catch {
    backendReady = false
  }
})

test('renders vm panel on machine detail', async ({ page }) => {
  test.skip(!backendReady, '本地后端未运行或缺少 machines 路由')
  await signIn(page)

  await page.goto('/machines')
  const firstRow = page.locator('tbody tr').first()
  await expect(firstRow).toBeVisible()
  await firstRow.click()
  await expect(page).toHaveURL(/\/machines\/[0-9a-f]{32}$/)

  // 面板标题与统计（无 VM 时显示空态）。
  await expect(page.getByRole('heading', { name: '虚拟机' })).toBeVisible()
  await expect(page.getByText(/共 \d+ 台虚拟机|没有虚拟机/).first()).toBeVisible()
})
