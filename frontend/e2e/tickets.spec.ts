import { expect, test, type Page } from '@playwright/test'

/**
 * 工单 e2e：登录 → 列表 → 新建 → 详情 → 评论 → 认领 → 解决 → 关闭 → 回列表检索。
 * 依赖本地后端（127.0.0.1:8080）与开发账号；后端不可用时整组跳过，避免 CI 误报。
 */

const username = process.env.E2E_USERNAME || 'admin'
const password = process.env.E2E_PASSWORD || 'Admin@12345'

let backendReady = false
let scopesReady = false
let inspectionsReady = false

async function signIn(page: Page) {
  await page.goto('/login')
  await page.getByLabel('账号').fill(username)
  await page.getByLabel('密码', { exact: true }).fill(password)
  await page.getByRole('button', { name: '登录', exact: true }).click()
  await expect(page.getByRole('heading', { name: '总览' })).toBeVisible()
}

test.beforeAll(async ({ request }) => {
  try {
    // 未登录访问工单接口：路由存在时返回 401/403，旧后端没有该路由时返回 404。
    const response = await request.get('/api/v1/admin/tickets')
    backendReady = response.status() !== 404
    const scopes = await request.get('/api/v1/admin/ticket-scopes')
    scopesReady = scopes.status() !== 404
    const inspections = await request.get('/api/v1/admin/inspection-plans')
    inspectionsReady = inspections.status() !== 404
  } catch {
    backendReady = false
    scopesReady = false
    inspectionsReady = false
  }
})

test.beforeEach(() => {
  test.skip(!backendReady, '后端未启动（/api/v1/healthz 不可用），跳过工单 e2e')
})

test('creates, comments and finishes a manual ticket', async ({ page }) => {
  await signIn(page)

  await page.goto('/tickets')
  await expect(page.getByRole('heading', { name: '工单', exact: true })).toBeVisible()

  // 新建工单（标题必填，分类 / 优先级可选项按当前 UI 选择）。
  const title = `dev-e2e-工单-${Date.now()}`
  await page.getByRole('button', { name: '新建工单' }).click()
  await page.getByLabel('标题').fill(title)
  await page.getByLabel('分类', { exact: true }).selectOption('other')
  await page.getByLabel('优先级', { exact: true }).selectOption('normal')
  await page.getByLabel('问题描述').fill('e2e：验证工单列表、详情、时间线、评论与状态流转。')
  await page.getByRole('button', { name: '创建工单' }).click()

  // 创建成功后直接进入详情页。
  await page.waitForURL(/\/tickets\/[0-9a-f-]+$/)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await expect(page.getByText(title)).toBeVisible()
  await expect(page.getByText('待处理', { exact: true }).first()).toBeVisible()
  await expect(page.getByRole('heading', { name: '附件' })).toBeVisible()

  // 评论：⌘/Ctrl+Enter 与按钮两条路径都走同一提交逻辑，这里验证按钮。
  await page.getByLabel('工单评论').fill('e2e：已确认现象，开始排查。')
  await page.getByRole('button', { name: '提交评论' }).click()
  await expect(page.getByText('e2e：已确认现象，开始排查。').first()).toBeVisible()

  // 认领 → 处理中。
  await page.getByRole('button', { name: '认领', exact: true }).click()
  await expect(page.getByText('处理中', { exact: true }).first()).toBeVisible()

  // ticket:manage：工单设置里转派（未分配）、改优先级、改分类。
  await page.getByRole('button', { name: '工单设置' }).click()
  await page.getByLabel('处理人', { exact: true }).selectOption('')
  await page.getByLabel('优先级', { exact: true }).selectOption('high')
  await page.getByLabel('分类', { exact: true }).selectOption('security')
  await page.getByRole('button', { name: '保存设置' }).click()
  await expect(page.getByText('工单设置已更新')).toBeVisible()
  await expect(page.getByText('安全', { exact: true }).first()).toBeVisible()

  // 标记解决必须填解决说明；根因分类可选（Wave 2）。
  await page.getByRole('button', { name: '标记解决' }).click()
  await page.getByLabel('解决说明').fill('e2e：重启 room-agent 后指标恢复，观察 10 分钟正常。')
  const resolveRootCause = page.getByLabel('根因分类')
  if (await resolveRootCause.isVisible().catch(() => false)) {
    await resolveRootCause.selectOption('hardware')
  }
  await page.getByRole('button', { name: '确认解决' }).click()
  await expect(page.getByText('已解决', { exact: true }).first()).toBeVisible()

  // 关闭工单（Wave 2：弹窗内可选根因分类与关闭备注）。
  await page.getByRole('button', { name: '关闭', exact: true }).filter({ hasText: '关闭' }).click()
  const closeRootCause = page.getByLabel('根因分类')
  if (await closeRootCause.isVisible().catch(() => false)) {
    await closeRootCause.selectOption('software')
  }
  await page.getByRole('button', { name: '确认关闭', exact: true }).click()
  await expect(page.getByText('已关闭', { exact: true }).first()).toBeVisible()

  // 回到列表，通过关键词与状态页签定位到刚关闭的工单。
  await page.getByRole('link', { name: '返回工单列表' }).click()
  await page.waitForURL('**/tickets')
  await page.getByRole('searchbox').fill(title)
  const row = page.locator('tbody tr', { hasText: title })
  await expect(row).toBeVisible()
  await expect(row.getByText('已关闭')).toBeVisible()
})

test('renders ticket insights with CSS chart', async ({ page }) => {
  await signIn(page)

  await page.goto('/tickets/insights')
  await expect(page.getByRole('heading', { name: '工单分析', exact: true })).toBeVisible()
  await expect(page.getByText('新增 vs 关闭（按日）')).toBeVisible()
  await expect(page.getByText('分类分布')).toBeVisible()
  await expect(page.getByText('房间 TOP5')).toBeVisible()
  await expect(page.getByText('处理人负载')).toBeVisible()

  // 切换统计窗口会重新拉取 stats（近 7 / 30 / 90 天）。
  await page.getByRole('button', { name: '近 7 天' }).click()
  await page.getByRole('button', { name: '近 90 天' }).click()
  await expect(page.getByRole('button', { name: '近 90 天' })).toBeVisible()
})

test('syncs list filters into the URL query', async ({ page }) => {
  await signIn(page)

  await page.goto('/tickets')
  await page.getByLabel('按优先级筛选工单').selectOption('high')
  await page.waitForURL(/priority=high/)
  await expect(page.getByLabel('按优先级筛选工单')).toHaveValue('high')

  // 刷新后从 query 恢复筛选。
  await page.reload()
  await expect(page.getByLabel('按优先级筛选工单')).toHaveValue('high')
})

test('renders ticket rules page and seeded rules', async ({ page }) => {
  await signIn(page)

  await page.goto('/tickets/rules')
  await expect(page.getByRole('heading', { name: '自动规则', exact: true })).toBeVisible()
  await expect(page.getByText('机器健康连续异常')).toBeVisible()
  await expect(page.getByText('设备离线').first()).toBeVisible()
  await expect(page.getByRole('button', { name: '新建规则' })).toBeVisible()
})

test('opens create-ticket modal prefilled from room 报修 entry', async ({ page }) => {
  await signIn(page)

  await page.goto('/rooms')
  await expect(page.getByRole('heading', { name: '房间', exact: true })).toBeVisible()
  await page.locator('tbody tr').first().click()
  await page.waitForURL(/\/rooms\/[0-9a-f-]+$/)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()

  await page.getByRole('button', { name: '报修 / 建单' }).click()
  await page.waitForURL(/\/tickets\?.*new=1/)
  await expect(page.getByRole('heading', { name: '新建工单' })).toBeVisible()

  const titleInput = page.getByLabel('标题')
  await expect(titleInput).toHaveValue(/房间报修：/)
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('heading', { name: '新建工单' })).toBeHidden()
})

test('renders ticket scopes page when backend supports it', async ({ page }) => {
  test.skip(!scopesReady, '后端尚未提供 /admin/ticket-scopes，跳过可见范围页 smoke')

  await signIn(page)
  await page.goto('/tickets/scopes')
  await expect(page.getByRole('heading', { name: '工单可见范围', exact: true })).toBeVisible()
  await expect(page.getByText('可见区域').first()).toBeVisible()
})

test('renders ticket inspections page when backend supports it', async ({ page }) => {
  test.skip(!inspectionsReady, '后端尚未提供 /admin/inspection-plans，跳过巡检计划页 smoke')

  await signIn(page)
  await page.goto('/tickets/inspections')
  await expect(page.getByRole('heading', { name: '巡检计划', exact: true })).toBeVisible()
  await expect(page.getByText('最近运行').first()).toBeVisible()

  await page.getByRole('button', { name: '新建计划' }).click()
  await expect(page.getByRole('heading', { name: '新建巡检计划' })).toBeVisible()
  await page.getByLabel('计划名称').fill(`dev-e2e-巡检-${Date.now()}`)
  await page.getByLabel('巡检范围', { exact: true }).selectOption('global')
  await page.getByLabel('巡检间隔').fill('24')
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.getByRole('heading', { name: '新建巡检计划' })).toBeHidden()
})
