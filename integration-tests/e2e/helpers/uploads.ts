import type { Page } from '@playwright/test'
import { expect } from '../fixtures'

export async function uploadFiles(page: Page, files: Parameters<ReturnType<Page['getByLabel']>['setInputFiles']>[0]) {
  await page.getByLabel('上传文件').locator('..').locator('input[type=file]').first().setInputFiles(files)
}

export async function confirmLatestUpload(page: Page, purpose: 'model_table' | 'data_resource', aliases: Record<string, string> = {}) {
  const confirm = page.getByRole('button', { name: '确认用途并发布' }).last()
  await expect(confirm).toBeVisible()
  const card = confirm.locator('xpath=ancestor::section[1]')
  const purposeSelects = card.getByRole('combobox', { name: /用途$/ })
  for (let index = 0; index < await purposeSelects.count(); index += 1) {
    await purposeSelects.nth(index).selectOption(purpose)
  }
  for (const [name, alias] of Object.entries(aliases)) {
    await card.getByRole('textbox', { name: new RegExp(`${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}.*alias$`) }).fill(alias)
  }
  const responsePromise = page.waitForResponse((response) => response.request().method() === 'POST' && /\/api\/upload-batches\/[^/]+\/confirm$/.test(new URL(response.url()).pathname))
  await confirm.click()
  const response = await responsePromise
  expect(response.status(), await response.text()).toBe(200)
}

export async function confirmLatestModelUpload(page: Page) {
  const confirm = page.getByRole('button', { name: '确认用途并发布' }).last()
  await expect(confirm).toBeVisible()
  const card = confirm.locator('xpath=ancestor::section[1]')
  const purposeSelects = card.getByRole('combobox', { name: /用途$/ })
  for (let index = 0; index < await purposeSelects.count(); index += 1) {
    const select = purposeSelects.nth(index)
    const displayName = String(await select.getAttribute('aria-label')).replace(/用途$/, '')
    const alias = displayName.replace(/\.parquet$/, '')
    await select.selectOption(alias === 'para' ? 'data_resource' : 'model_table')
    if (displayName.endsWith('.parquet')) {
      await card.getByRole('textbox', { name: `${displayName} alias` }).fill(alias)
    }
  }
  const responsePromise = page.waitForResponse((response) => response.request().method() === 'POST' && /\/api\/upload-batches\/[^/]+\/confirm$/.test(new URL(response.url()).pathname))
  await confirm.click()
  const response = await responsePromise
  expect(response.status(), await response.text()).toBe(200)
}
