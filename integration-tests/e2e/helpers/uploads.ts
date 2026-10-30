import type { Page } from '@playwright/test'
import { expect } from '../fixtures'

export async function uploadFiles(page: Page, files: Parameters<ReturnType<Page['getByLabel']>['setInputFiles']>[0]) {
  await page.getByLabel('上传文件').locator('..').locator('input[type=file]').first().setInputFiles(files)
}

export async function openLatestUploadBatch(page: Page) {
  const open = page.getByRole('button', { name: /前往(?:模型工作区|数据文件)/ }).last()
  await expect(open).toBeVisible()
  await open.click()
}

export async function openMutableDraft(page: Page) {
  const expandPanel = page.getByRole('button', { name: '展开结果面板' })
  if (await expandPanel.isVisible()) await expandPanel.click()
  await page.getByRole('button', { name: '模型工作区' }).click()
  const draft = page.getByRole('button', { name: /^(?:展开|收起)可变草稿/ })
  await expect(draft).toBeVisible()
  if (await draft.getAttribute('aria-expanded') === 'false') await draft.click()
  return draft
}

export async function confirmLatestUpload(page: Page, purpose: 'model_table' | 'data_resource', aliases: Record<string, string> = {}) {
  await openLatestUploadBatch(page)
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
  await openLatestUploadBatch(page)
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
