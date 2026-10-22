import type { APIRequestContext } from '@playwright/test'
import { test, expect } from './fixtures'
import { directApi } from './helpers/server'

async function createSession(api: APIRequestContext, title: string) {
  const response = await api.post('/api/sessions', { data: { title } })
  expect(response.status()).toBe(201)
  return response.json() as Promise<{ id: string }>
}

test('two owners cannot enumerate or guess each other session, message, SSE, task, or dataset resources', async ({ appPage, control }) => {
  const alice = await appPage({ subject: 'alice@example.com', tenant: 'shared-grid' })
  const bob = await appPage({ subject: 'bob@example.com', tenant: 'shared-grid' })
  const aliceSession = await createSession(alice.api, 'Alice 私有会话')
  const bobSession = await createSession(bob.api, 'Bob 私有会话')

  const aliceList = await alice.api.get('/api/sessions')
  const bobList = await bob.api.get('/api/sessions')
  expect((await aliceList.json()).sessions.map((session: { id: string }) => session.id)).toEqual([aliceSession.id])
  expect((await bobList.json()).sessions.map((session: { id: string }) => session.id)).toEqual([bobSession.id])

  await control.enqueueLlm({
    id: 'alice-import',
    expectedPrompt: '导入隔离案例',
    turns: [
      {
        kind: 'tool_call', call_id: 'import-alice', name: 'import_example',
        args: { exampleId: 'e2e-demo', engineResourceSnapshot: 'e2e-snapshot', expectedDraftVersionId: null },
      },
      { kind: 'text', text: '案例导入完成' },
    ],
  })
  expect((await alice.api.post(`/api/sessions/${aliceSession.id}/messages`, { data: { message: '导入隔离案例' } })).status()).toBe(202)
  let versionId = ''
  await expect.poll(async () => {
    const workspace = await (await alice.api.get(`/api/sessions/${aliceSession.id}/workspace`)).json()
    versionId = workspace.draft?.versionId ?? workspace.active?.versionId ?? ''
    return versionId
  }).not.toBe('')

  await control.enqueueEngine({ kind: 'success', data: [{ name: 'voltages', rows: [['bus-1', 1.0]], valueUnit: 'pu' }] })
  await control.enqueueLlm({
    id: 'alice-study',
    expectedPrompt: '运行隔离潮流',
    turns: [
      { kind: 'tool_call', call_id: 'skill-1', name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
      { kind: 'tool_call', call_id: 'skill-2', name: 'load_agent_skill', args: { skill_id: 'rss-generation' } },
      {
        kind: 'tool_call', call_id: 'run-alice', name: 'run_study',
        args: { modelVersionId: Number(versionId), studyType: 'power_flow', studySpec: {}, script: 'result = 1\n' },
      },
      { kind: 'text', text: '隔离潮流完成' },
    ],
  })
  expect((await alice.api.post(`/api/sessions/${aliceSession.id}/messages`, { data: { message: '运行隔离潮流' } })).status()).toBe(202)
  let taskId = ''
  await expect.poll(async () => {
    const tasks = await (await alice.api.get(`/api/sessions/${aliceSession.id}/tasks`)).json()
    taskId = tasks[0]?.id ?? ''
    return tasks[0]?.status ?? ''
  }).toBe('succeeded')
  const aliceDatasets = await (await alice.api.get(`/api/tasks/${taskId}/datasets`)).json()
  expect(aliceDatasets).toHaveLength(1)
  const datasetId = aliceDatasets[0].id as string

  const hiddenPaths = [
    `/api/sessions/${aliceSession.id}/messages`,
    `/api/sessions/${aliceSession.id}/events`,
    `/api/sessions/${aliceSession.id}/tasks`,
    `/api/tasks/${taskId}`,
    `/api/tasks/${taskId}/datasets`,
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

test('trusted proxy rejects missing and incorrect credentials', async () => {
  const missing = await directApi('/api/me', {})
  expect(missing.status).toBe(401)
  const wrong = await directApi('/api/me', { subject: 'mallory@example.com', token: 'wrong-token' })
  expect(wrong.status).toBe(401)
})
