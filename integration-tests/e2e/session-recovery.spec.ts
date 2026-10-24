import type { APIRequestContext } from '@playwright/test'
import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { readEvent } from './helpers/events'
import { restartBackend } from './helpers/server'

async function createSession(api: APIRequestContext, title: string) {
  const response = await api.post('/api/sessions', { data: { title } })
  expect(response.status()).toBe(201)
  return response.json() as Promise<{ id: string; title: string }>
}

async function messages(api: APIRequestContext, sessionId: string) {
  const response = await api.get(`/api/sessions/${sessionId}/messages?limit=100`)
  expect(response.ok()).toBeTruthy()
  return response.json() as Promise<{ messages: Array<{ id: string; type: string; content?: string }> }>
}

test('an empty account bootstraps a session before message, upload, and example actions', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'empty-session@example.com' })
  const createResponse = page.waitForResponse((response) =>
    response.url().endsWith('/api/sessions') && response.request().method() === 'POST', { timeout: 10_000 })
  await page.goto('/')
  const createdResponse = await createResponse
  expect(createdResponse.status()).toBe(201)
  const created = await createdResponse.json() as { id: string }
  await expect.poll(async () => {
    const sessions = (await (await api.get('/api/sessions')).json()).sessions as Array<{ id: string }>
    return sessions.map(session => session.id)
  }).toEqual([created.id])

  await control.enqueueLlm({
    id: 'empty-session-message',
    expectedPrompt: '检查空账户会话',
    turns: [{ kind: 'text', text: '空账户会话已就绪' }],
  })
  await page.getByRole('textbox', { name: '消息' }).fill('检查空账户会话')
  await page.getByRole('button', { name: '发送消息' }).click()
  await expect(page.getByRole('heading', { name: '检查空账户会话' })).toBeVisible()
  await expect(page.getByText('空账户会话已就绪', { exact: true })).toBeVisible()

  await page.locator('input[type=file]').first().setInputFiles(
    resolve(process.cwd(), '../integration-tests/fixtures/uploads/rss/replacement.rss'),
  )
  await expect(page.getByRole('article', { name: '脚本 replacement.rss' })).toBeVisible()

  await page.getByRole('button', { name: '案例库' }).click()
  await page.getByRole('textbox', { name: '搜索内置案例' }).fill('E2E')
  await page.getByRole('button', { name: 'E2E demo', exact: true }).click()
  await page.getByRole('button', { name: '使用案例“E2E demo”' }).click()
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()

  const deleted = page.waitForResponse((response) =>
    response.url().endsWith(`/api/sessions/${created.id}`) && response.request().method() === 'DELETE')
  const replacement = page.waitForResponse((response) =>
    response.url().endsWith('/api/sessions') && response.request().method() === 'POST', { timeout: 10_000 })
  await page.getByRole('button', { name: '打开“检查空账户会话”的会话操作' }).click()
  await page.getByRole('button', { name: '删除“检查空账户会话”' }).click()
  await page.getByRole('button', { name: '确认删除' }).click()
  expect((await deleted).status()).toBe(204)
  const replacementResponse = await replacement
  expect(replacementResponse.status()).toBe(201)
  const replacementSession = await replacementResponse.json() as { id: string }
  await expect.poll(async () => {
    const sessions = (await (await api.get('/api/sessions')).json()).sessions as Array<{ id: string }>
    return sessions.map(session => session.id)
  }).toEqual([replacementSession.id])

  await control.enqueueLlm({
    id: 'replacement-session-message',
    expectedPrompt: '检查替代会话',
    turns: [{ kind: 'text', text: '替代会话已就绪' }],
  })
  await page.getByRole('textbox', { name: '消息' }).fill('检查替代会话')
  await page.getByRole('button', { name: '发送消息' }).click()
  await expect(page.getByText('替代会话已就绪', { exact: true })).toBeVisible()
})

test('session bootstrap blocks primary actions and provides a persistent retry', async ({ appPage }) => {
  const { page } = await appPage({ subject: 'bootstrap-error@example.com' })
  let release!: () => void
  const gate = new Promise<void>(resolveGate => { release = resolveGate })
  let allowSuccess = false
  await page.route('**/api/sessions', async route => {
    if (route.request().method() === 'GET' && !allowSuccess) {
      await gate
      await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: '会话初始化失败' }) })
      return
    }
    await route.continue()
  })
  await page.goto('/')
  await expect(page.getByText('正在准备会话…', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '案例库' })).toHaveCount(0)
  release()
  await expect(page.getByRole('alert')).toContainText('会话初始化失败')
  allowSuccess = true
  await page.getByRole('button', { name: '重试' }).click()
  await expect(page.getByRole('textbox', { name: '消息' })).toBeVisible()
})

test('session CRUD renders optimistic user, tool-call, and assistant messages', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'session-crud@example.com', tenant: 'grid-a' })
  expect((await api.get('/health')).status()).toBe(200)
  await page.goto('/')
  await expect(page.getByRole('button', { name: '新建仿真' })).toBeVisible()

  const createResponse = page.waitForResponse((response) => response.url().endsWith('/api/sessions') && response.request().method() === 'POST')
  await page.getByRole('button', { name: '新建仿真' }).click()
  const created = await (await createResponse).json() as { id: string }
  await page.getByRole('button', { name: '打开“新会话”的会话操作' }).first().click()
  await page.getByRole('button', { name: '重命名“新会话”' }).click()
  await page.getByRole('textbox', { name: '重命名“新会话”' }).fill('馈线恢复测试')
  await page.getByRole('textbox', { name: '重命名“新会话”' }).press('Enter')
  await expect(page.getByRole('heading', { name: '馈线恢复测试' })).toBeVisible()

  const other = await createSession(api, '切换目标')
  await page.reload()
  await page.getByRole('button', { name: /打开会话.*馈线恢复测试/ }).click()
  await expect(page.getByRole('heading', { name: '馈线恢复测试' })).toBeVisible()
  await page.getByRole('button', { name: /打开会话.*切换目标/ }).getByText('已完成').click()
  await expect(page.getByRole('heading', { name: '切换目标' })).toBeVisible()
  await page.getByRole('button', { name: /打开会话.*馈线恢复测试/ }).click()

  await control.enqueueLlm({
    id: 'browser-message-types',
    expectedPrompt: '检查当前工作区',
    turns: [
      { kind: 'delay_until_released', barrier_id: 'optimistic-user' },
      { kind: 'tool_call', call_id: 'inspect-1', name: 'inspect_workspace', args: {} },
      { kind: 'text', text: '工作区检查完成' },
    ],
  })
  let continuePost!: () => void
  const postGate = new Promise<void>((resolve) => { continuePost = resolve })
  await page.route(`**/api/sessions/${created.id}/messages`, async (route) => {
    await postGate
    await route.continue()
  }, { times: 1 })
  await page.getByRole('textbox', { name: '消息' }).fill('检查当前工作区')
  await page.getByRole('button', { name: '发送消息' }).click()
  await expect(page.getByTestId('message-user')).toHaveCount(1)
  await expect(page.getByTestId('message-user')).toHaveAttribute('data-message-id', /^local-user-/)
  continuePost()
  await expect(page.getByTestId('message-user')).toHaveAttribute('data-message-id', /^\d+$/)
  await control.release('optimistic-user')
  await expect(page.getByTestId('message-tool-call')).toContainText('inspect_workspace')
  await expect(page.getByTestId('message-assistant')).toContainText('工作区检查完成')
  await expect(page.getByTestId('message-user')).toHaveCount(1)

  await page.getByRole('textbox', { name: '搜索历史' }).fill('检查当前工作区')
  await expect(page.getByRole('button', { name: '打开历史会话“馈线恢复测试”' })).toBeVisible()
  await page.getByRole('button', { name: '打开历史会话“馈线恢复测试”' }).click()
  await expect(page.getByTestId('message-assistant')).toContainText('工作区检查完成')

  await page.getByRole('button', { name: '打开“切换目标”的会话操作' }).click()
  await page.getByRole('button', { name: '删除“切换目标”' }).click()
  await page.getByRole('button', { name: '确认删除' }).click()
  await expect(page.getByRole('button', { name: /打开会话.*切换目标/ })).toHaveCount(0)
  expect((await messages(api, created.id)).messages.map((item) => item.type)).toEqual(
    expect.arrayContaining(['user', 'tool-call', 'assistant']),
  )
  expect(other.id).not.toBe(created.id)
})

test('older history, refresh, restart, and invalid localStorage recover committed state', async ({ appPage, control }) => {
  const identity = { subject: 'history@example.com', tenant: 'grid-a' }
  const { page, api } = await appPage(identity)
  const session = await createSession(api, '超过五十条消息')
  const baseline = ((await control.trace()) as { agent: unknown[] }).agent.length
  for (let index = 0; index < 26; index += 1) {
    await control.enqueueLlm({
      id: `history-${index}`,
      expectedPrompt: `批量消息 ${index}`,
      turns: [{ kind: 'text', text: `批量回复 ${index}` }],
    })
    const response = await api.post(`/api/sessions/${session.id}/messages`, { data: { message: `批量消息 ${index}` } })
    expect(response.status()).toBe(202)
    await expect.poll(async () => ((await control.trace()) as { agent: unknown[] }).agent.length).toBe(baseline + index + 1)
    await expect.poll(async () => (await messages(api, session.id)).messages
      .filter((message: { type: string }) => message.type === 'assistant').length).toBe(index + 1)
  }
  await expect.poll(async () => (await messages(api, session.id)).messages.reduce<Record<string, number>>((counts, message) => {
    counts[message.type] = (counts[message.type] ?? 0) + 1
    return counts
  }, {})).toEqual({ user: 26, assistant: 26 })

  await page.goto('/')
  await expect(page.getByRole('heading', { name: '批量消息 0' })).toBeVisible()
  await expect(page.getByRole('button', { name: '加载更早消息' })).toBeVisible()
  await expect(page.locator('[data-message-id]')).toHaveCount(50)
  await page.getByRole('button', { name: '加载更早消息' }).click()
  await expect(page.locator('[data-message-id]')).toHaveCount(52)

  await page.reload()
  await expect(page.getByText('批量回复 25', { exact: true })).toHaveCount(1)
  await restartBackend()
  await page.reload()
  await expect(page.getByText('批量回复 25', { exact: true })).toHaveCount(1)

  await page.evaluate(() => localStorage.setItem('zjudss:last-session:tenant:grid-a:owner:history@example.com', '2147483647'))
  await page.reload()
  await expect(page.getByRole('heading', { name: '批量消息 0' })).toBeVisible()
})

test('SSE cursor replays committed messages, requests resync, and does not duplicate DOM rows', async ({ appPage, control }) => {
  const identity = { subject: 'sse@example.com', tenant: 'grid-a' }
  const first = await appPage(identity)
  const session = await createSession(first.api, 'SSE 恢复')
  await first.page.goto('/')
  await control.enqueueLlm({
    id: 'disconnect-after-commit',
    expectedPrompt: '断线恢复请求',
    turns: [
      { kind: 'delay_until_released', barrier_id: 'disconnect' },
      { kind: 'text', text: '断线后完成的唯一回复' },
    ],
  })
  await first.page.getByRole('textbox', { name: '消息' }).fill('断线恢复请求')
  await first.page.getByRole('button', { name: '发送消息' }).click()
  await expect(first.page.getByTestId('message-user')).toHaveCount(1)
  const persisted = await messages(first.api, session.id)
  const cursor = persisted.messages.find((message) => message.type === 'user')?.id
  expect(cursor).toBeTruthy()
  await first.page.close()
  await control.release('disconnect')
  await expect.poll(async () => (await messages(first.api, session.id)).messages.some((message) => message.content === '断线后完成的唯一回复')).toBe(true)

  const replay = await readEvent(`/api/sessions/${session.id}/events`, identity, cursor)
  expect(replay).toContain('event: message')
  expect(replay).toContain('断线后完成的唯一回复')
  const resync = await readEvent(`/api/sessions/${session.id}/events`, identity, '2147483647', 'resync-required')
  expect(resync).toContain('event: resync-required')

  const recovered = await appPage(identity)
  await recovered.page.goto('/')
  await expect(recovered.page.getByText('断线后完成的唯一回复', { exact: true })).toHaveCount(1)
  await recovered.page.reload()
  await expect(recovered.page.getByText('断线后完成的唯一回复', { exact: true })).toHaveCount(1)
})
