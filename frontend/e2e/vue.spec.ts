import { test, expect, type Page } from '@playwright/test'

const username = process.env.E2E_USERNAME || 'admin'
const password = process.env.E2E_PASSWORD || 'Admin@12345'

async function signIn(page: Page) {
  await page.goto('/login')
  await page.getByLabel('账号').fill(username)
  await page.getByLabel('密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '登录', exact: true }).click()
  await expect(page.getByRole('heading', { name: '总览' })).toBeVisible()
}

test('anonymous visitors land on the login page', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: '登录', exact: true })).toBeVisible()
  await expect(page).toHaveURL(/\/login/)
})

test('signs in and renders the overview workspace', async ({ page }) => {
  await signIn(page)
  await expect(page.getByRole('heading', { name: '总览' })).toBeVisible()
  await expect(page.getByText('系统状态')).toBeVisible()
})

test('navigates to rooms and devices pages', async ({ page }) => {
  await signIn(page)

  await page.getByRole('link', { name: '房间' }).click()
  await page.waitForURL('**/rooms')
  await expect(page.getByRole('heading', { name: '房间', exact: true })).toBeVisible()

  await page.getByRole('link', { name: '设备' }).click()
  await page.waitForURL('**/devices')
  await expect(page.getByRole('heading', { name: '设备', exact: true })).toBeVisible()
})

test('opens area overview and machine overview', async ({ page }) => {
  await signIn(page)

  await page.goto('/areas')
  await expect(page.getByRole('heading', { name: '区域', exact: true })).toBeVisible()
  await page.locator('tbody tr').first().getByRole('link').first().click()
  await page.waitForURL(/\/areas\/[0-9a-f-]+$/)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()

  await page.goto('/machines')
  await expect(page.getByRole('heading', { name: '机器总览', exact: true })).toBeVisible()
  await page.locator('tbody tr').first().click()
  await page.waitForURL(/\/machines\/[0-9a-f]+$/)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await expect(page.getByText('机器身份')).toBeVisible()
  // 进入页面即建立 SSE，右上角应出现实时流状态（连接中/已连接均可）。
  await expect(page.getByText(/实时流/).first()).toBeVisible()
})
