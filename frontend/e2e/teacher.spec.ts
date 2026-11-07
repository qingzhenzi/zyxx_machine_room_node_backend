import { expect, test, type Page } from '@playwright/test'

/**
 * 教师工作台（阶段1）e2e：登录 → 侧栏入口 → /teacher 页面主要区块渲染。
 * 依赖本地后端（127.0.0.1:8080）与开发账号；后端不可用时整组跳过。
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
    // 未登录访问接口：路由存在时返回 401/403，旧后端没有该路由时返回 404。
    const response = await request.get('/api/v1/admin/machines')
    backendReady = response.status() !== 404
  } catch {
    backendReady = false
  }
})

test('renders teacher workspace and quick actions', async ({ page }) => {
  test.skip(!backendReady, '本地后端未运行或缺少 machines 路由')
  await signIn(page)

  // 侧栏入口（工作台分组）
  await expect(page.getByRole('link', { name: '教师工作台' })).toBeVisible()

  await page.getByRole('link', { name: '教师工作台' }).click()
  await expect(page).toHaveURL(/\/teacher$/)
  await expect(page.getByRole('heading', { name: '教师工作台' })).toBeVisible()

  // 三个主要区块：今日课程 / 我的房间 / 快捷操作（实验环境模板占位）
  await expect(page.getByRole('heading', { name: '今日课程' })).toBeVisible()
  await expect(page.getByRole('heading', { name: '我的房间' })).toBeVisible()
  await expect(page.getByRole('heading', { name: '实验环境模板' })).toBeVisible()
  await expect(page.getByText('即将上线').first()).toBeVisible()
})
