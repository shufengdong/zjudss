import AxeBuilder from '@axe-core/playwright'
import { test, expect, type Page } from './fixtures'
import { createReadySession, uploadScriptAndRun } from './helpers/journeys'

async function expectNoSeriousViolations(page: Page) {
  const result = await new AxeBuilder({ page }).analyze()
  expect(result.violations.filter(violation => violation.impact === 'serious' || violation.impact === 'critical')).toEqual([])
}

async function expectUserMenuAtSidebarBottom(page: Page, userName: string) {
  const menu = page.getByRole('button', { name: new RegExp(userName) })
  await expect(menu).toBeInViewport()
  const box = await menu.boundingBox()
  expect(box).not.toBeNull()
  const bottomGap = page.viewportSize()!.height - box!.y - box!.height
  expect(bottomGap).toBeGreaterThanOrEqual(0)
  expect(bottomGap).toBeLessThanOrEqual(12)
}

async function openReplacementRunResult(page: Page) {
  const runHeader = page.getByRole('button', { name: /replacement\.rss.*成功/ })
  const runCard = runHeader.locator('xpath=ancestor::article[1]')
  const details = runCard.getByRole('button', { name: '展开运行详情' })
  await expect(details).toHaveAttribute('aria-expanded', 'false')
  await details.click()
  await runCard.getByRole('button', { name: '查看 voltage' }).click()
}

test('desktop three-column workspace completes upload, script, run, and result flows with accessible controls', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'desktop@example.com', tenant: 'grid-ui' })
  await page.setViewportSize({ width: 1440, height: 900 })
  const { sessionId } = await createReadySession(page, api, '桌面工作台')
  await expect(page.getByRole('button', { name: '新建仿真' })).toBeVisible()
  await expect(page.getByRole('textbox', { name: '消息' })).toBeVisible()
  await expect(page.getByRole('button', { name: '模型工作区' })).toBeVisible()
  await expectUserMenuAtSidebarBottom(page, 'Desktop Test')
  await uploadScriptAndRun({
    page, api, control, sessionId,
    engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1]], valueUnit: 'pu' }] },
  })
  await page.getByRole('button', { name: '运行记录' }).click()
  await openReplacementRunResult(page)
  await expect(page.getByRole('columnheader', { name: 'value (pu)' })).toBeVisible()
  await expectNoSeriousViolations(page)
})

test('responsive narrow drawers and model tables remain keyboard reachable without horizontal overflow', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'narrow@example.com', tenant: 'grid-ui' })
  await page.setViewportSize({ width: 390, height: 844 })
  const { sessionId } = await createReadySession(page, api, '窄屏工作台')
  await expect(page.getByRole('button', { name: '展开历史侧栏' })).toBeVisible()
  await expect(page.getByRole('button', { name: '展开结果面板' })).toBeVisible()
  await page.getByRole('button', { name: '展开历史侧栏' }).click()
  await expect(page.getByRole('textbox', { name: '搜索历史' })).toBeVisible()
  await expectUserMenuAtSidebarBottom(page, 'Narrow Test')
  await page.getByRole('button', { name: '收起历史侧栏' }).click()

  await page.getByRole('button', { name: '展开结果面板' }).click()
  await page.getByRole('button', { name: /^展开当前模型/ }).click()
  await page.getByRole('button', { name: '查看 bus' }).click()
  await expect(page.getByRole('dialog', { name: /当前模型 · 版本 .* · bus/ })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await expectNoSeriousViolations(page)
  await page.getByRole('button', { name: '关闭模型表' }).click()
  await page.getByRole('button', { name: '关闭结果面板' }).click()

  await uploadScriptAndRun({
    page, api, control, sessionId,
    engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1]], valueUnit: 'pu' }] },
  })
  await page.getByRole('button', { name: '展开结果面板' }).click()
  await page.getByRole('button', { name: '运行记录' }).click()
  await openReplacementRunResult(page)
  await expect(page.getByRole('columnheader', { name: 'value (pu)' })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await expectNoSeriousViolations(page)
  await page.getByRole('button', { name: '关闭数据集' }).click()
  await page.getByRole('button', { name: '关闭结果面板' }).click()
  await page.reload()
  await expect(page.getByRole('button', { name: '展开历史侧栏' })).toBeVisible()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: '展开历史侧栏' })).toBeFocused()
})
