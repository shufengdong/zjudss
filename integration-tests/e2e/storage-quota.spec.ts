import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'
import { confirmLatestUpload } from './helpers/uploads'

test('warning and hard quota are visible while a rejected model import leaves usage and workspace intact', async ({ appPage, control }) => {
  const identity = { subject: 'quota@example.com', tenant: 'grid-q' }
  const { page, api } = await appPage(identity)
  const sessionId = await createSessionFromPage(page, '存储配额')
  await control.setStorageLimits({ kind: 'user', key: 'quota@example.com' }, { artifactLimit: 10, byteLimit: 1024 * 1024, warningRatio: 0.8 })
  const input = page.locator('input[type=file]').first()
  await input.setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/bus.csv'))
  await confirmLatestUpload(page, 'model_table')
  await page.getByRole('button', { name: /^展开可变草稿/ }).click()
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.id).toBeTruthy()
  const committedUsage = await (await api.get('/api/storage-usage')).json()
  await control.setStorageLimits(
    { kind: 'user', key: 'quota@example.com' },
    { artifactLimit: committedUsage.artifactCount, byteLimit: 1024 * 1024, warningRatio: 0.8 },
  )
  await page.reload()
  await expect(page.getByText(/存储已使用/)).toBeVisible()
  const before = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  expect(before.active?.id).toBeTruthy()

  const rejected = page.waitForResponse(response => /\/api\/sessions\/[^/]+\/upload-batches$/.test(new URL(response.url()).pathname) && response.request().method() === 'POST')
  await input.setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.csv'))
  const accepted = await rejected
  expect(accepted.status()).toBe(202)
  const batchId = ((await accepted.json()) as { uploadBatch: { id: string } }).uploadBatch.id
  await expect.poll(async () => (await (await api.get(`/api/upload-batches/${batchId}`)).json()).uploadBatch.status).toBe('failed')
  expect((await (await api.get(`/api/upload-batches/${batchId}`)).json()).uploadBatch.diagnostics.errors[0].code).toBe('storage_quota_exceeded')
  await expect(page.getByText(/存储配额不足/)).toBeVisible()
  const usage = await (await api.get('/api/storage-usage')).json()
  expect(usage.artifactCount).toBe(committedUsage.artifactCount)
  const after = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  expect(after.active.id).toBe(before.active.id)
  expect(after.draft).toBeNull()
})
