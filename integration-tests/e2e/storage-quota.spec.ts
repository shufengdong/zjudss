import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'

test('warning and hard quota are visible while rejected ingestion leaves usage and workspace intact', async ({ appPage, control }) => {
  const identity = { subject: 'quota@example.com', tenant: 'grid-q' }
  const { page, api } = await appPage(identity)
  const sessionId = await createSessionFromPage(page, '存储配额')
  await control.setStorageLimits(
    { kind: 'owner', key: 'tenant:grid-q:owner:quota@example.com' },
    { artifactLimit: 1, byteLimit: 1024 * 1024, warningRatio: 0.8 },
  )
  const input = page.locator('input[type=file]').first()
  await input.setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/bus.csv'))
  await expect.poll(async () => (await (await api.get('/api/storage-usage')).json()).artifactCount).toBe(1)
  await expect(page.getByText(/存储已使用/)).toBeVisible()
  const before = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  expect(before.active?.status).toBe('ready')

  await input.setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.csv'))
  await expect(page.getByText('failed', { exact: true })).toBeVisible()
  await expect(page.getByText(/存储配额不足/)).toBeVisible()
  const usage = await (await api.get('/api/storage-usage')).json()
  expect(usage.artifactCount).toBe(1)
  const after = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  expect(after.active.versionId).toBe(before.active.versionId)
  expect(after.draft).toBeNull()
})
