import type { APIRequestContext } from '@playwright/test'
import { test, expect } from './fixtures'
import { directApi, resetBackend } from './helpers/server'
import { createReadySession, uploadScriptAndRun } from './helpers/journeys'

async function createSession(api: APIRequestContext, title: string) {
  const response = await api.post('/api/sessions', { data: { title } })
  expect(response.status()).toBe(201)
  return response.json() as Promise<{ id: string }>
}

test('two users cannot enumerate or guess each other session, message, SSE, run, or dataset resources', async ({ loginAs, control }) => {
  const alice = await loginAs('alice@example.com')
  const bob = await loginAs('bob@example.com')
  const { sessionId: aliceSessionId } = await createReadySession(alice.page, alice.api, 'Alice 私有会话')
  const bobSession = await createSession(bob.api, 'Bob 私有会话')

  const aliceList = await alice.api.get('/api/sessions')
  const bobList = await bob.api.get('/api/sessions')
  const aliceSessionIds = (await aliceList.json()).sessions.map((session: { id: string }) => session.id)
  const bobSessionIds = (await bobList.json()).sessions.map((session: { id: string }) => session.id)
  expect(aliceSessionIds).toContain(aliceSessionId)
  expect(aliceSessionIds).not.toContain(bobSession.id)
  expect(bobSessionIds).toEqual([bobSession.id])
  expect(bobSessionIds).not.toContain(aliceSessionId)

  const { run } = await uploadScriptAndRun({
    page: alice.page, api: alice.api, control, sessionId: aliceSessionId,
    engine: { kind: 'success', data: [{ name: 'voltages', rows: [['bus-1', 1.0]], valueUnit: 'pu' }] },
  })
  const aliceDatasets = (await (await alice.api.get(`/api/runs/${run.id}/datasets`)).json()).datasets
  expect(aliceDatasets).toHaveLength(1)
  const datasetId = aliceDatasets[0].id as string

  const hiddenPaths = [
    `/api/sessions/${aliceSessionId}/messages`,
    `/api/sessions/${aliceSessionId}/events`,
    `/api/sessions/${aliceSessionId}/runs`,
    `/api/runs/${run.id}`,
    `/api/runs/${run.id}/datasets`,
    `/api/datasets/${datasetId}`,
    `/api/datasets/${datasetId}/rows`,
    `/api/datasets/${datasetId}/download`,
  ]
  for (const path of hiddenPaths) {
    const hidden = await bob.api.get(path)
    expect(hidden.status(), path).toBe(404)
  }

  const history = await bob.api.get('/api/history/search?query=Alice%20私有会话')
  expect(history.ok()).toBeTruthy()
  expect((await history.json()).items).toEqual([])
})

test('cookie login rejects missing and incorrect credentials', async () => {
  const missing = await directApi('/api/me')
  expect(missing.status).toBe(401)
  const wrong = await fetch('http://127.0.0.1:18080/api/auth/login', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ id: 'alice@example.com', password: 'wrong' }),
  })
  expect(wrong.status).toBe(401)
})

test('login persists across refresh and logout removes stale user state', async ({ loginAs }) => {
  const alice = await loginAs('alice@example.com')
  await createSession(alice.api, 'Alice 不应泄漏')
  await alice.page.goto('/')
  await expect(alice.page.getByText('Alice', { exact: true })).toBeVisible()
  await expect(alice.page.getByRole('button', { name: '打开会话“Alice 不应泄漏”' })).toBeVisible()
  await alice.page.reload()
  await expect(alice.page.getByText('Alice', { exact: true })).toBeVisible()
  await alice.page.goto('/?history-check=1')
  await expect(alice.page.getByText('Alice', { exact: true })).toBeVisible()

  await alice.page.getByRole('button', { name: 'A Alice' }).click()
  await alice.page.getByRole('button', { name: '退出登录' }).click()
  await expect(alice.page.getByLabel('用户 ID')).toBeVisible()
  await alice.page.goBack()
  await expect(alice.page.getByLabel('用户 ID')).toBeVisible()

  await alice.page.getByLabel('用户 ID').fill('bob@example.com')
  await alice.page.getByLabel('密码').fill('bob@example.com')
  await alice.page.getByRole('button', { name: '登录' }).click()
  await expect(alice.page.getByText('Bob', { exact: true })).toBeVisible()
  await expect(alice.page.getByText('Alice 不应泄漏', { exact: true })).toHaveCount(0)
})

test('database reset invalidates an existing browser cookie', async ({ loginAs }) => {
  const alice = await loginAs('alice@example.com')
  expect((await alice.api.get('/api/me')).status()).toBe(200)
  await resetBackend()
  expect((await alice.api.get('/api/me')).status()).toBe(401)
})
