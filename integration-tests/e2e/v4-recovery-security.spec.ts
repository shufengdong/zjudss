import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'
import { newtonScript, prepareNewtonStudy, publishExampleModel } from './helpers/managedStudies'

test('a blocked managed workflow can be cancelled, recovered after refresh, and remains user scoped', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'alice@example.com' })
  const sessionId = await createSessionFromPage(page, '取消与恢复')
  await publishExampleModel(page, api, sessionId)
  const configuration = await prepareNewtonStudy(page, api, sessionId, control)
  await control.enqueueAuthor({ kind: 'wait', barrier_id: 'cancel-author' })
  await control.enqueueAuthor({ kind: 'script', content: newtonScript })
  const authorBaseline = (await control.trace() as { author: string[] }).author.length

  const confirmedResponse = page.waitForResponse(response => response.request().method() === 'POST' && /\/api\/study-configurations\/\d+\/confirm$/.test(new URL(response.url()).pathname))
  await configuration.card.getByRole('button', { name: '确认并生成脚本' }).click()
  const confirmed = await confirmedResponse
  expect(confirmed.status()).toBe(200)
  const workflow = await confirmed.json() as { id: number; runId: number }
  await expect.poll(async () => ((await control.trace() as { author: string[] }).author.length)).toBe(authorBaseline + 1)

  const card = page.getByRole('article', { name: '研究工作流进度' }).last()
  const cancelResponse = page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === `/api/workflows/${workflow.id}/cancel`)
  await card.getByRole('button', { name: '取消' }).click()
  expect((await cancelResponse).status()).toBe(200)
  await control.release('cancel-author')
  await expect.poll(async () => ((await (await api.get(`/api/workflows/${workflow.id}`)).json()) as { status: string }).status).toBe('cancelled_by_user')

  await page.reload()
  await expect(page.getByRole('article', { name: '研究工作流进度' }).last()).toContainText('cancelled_by_user')
  const { api: foreignApi } = await appPage({ subject: 'bob@example.com' })
  expect((await foreignApi.get(`/api/workflows/${workflow.id}`)).status()).toBe(404)

  const preview = await api.post(`/api/runs/${workflow.runId}/deletion-preview`)
  expect(preview.status()).toBe(200)
  const impact = await preview.json() as { impactToken: string }
  const deleted = await api.delete(`/api/runs/${workflow.runId}?impactToken=${encodeURIComponent(impact.impactToken)}`)
  expect(deleted.status(), await deleted.text()).toBe(204)
  expect((await api.get(`/api/runs/${workflow.runId}`)).status()).toBe(404)
})
