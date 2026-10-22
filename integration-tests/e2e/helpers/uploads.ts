import type { Page } from '@playwright/test'

export async function uploadFiles(page: Page, files: Parameters<ReturnType<Page['getByLabel']>['setInputFiles']>[0]) {
  await page.getByLabel('上传文件').locator('..').locator('input[type=file]').first().setInputFiles(files)
}
