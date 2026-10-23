import { resolve } from 'node:path'
import type { APIRequestContext, Page } from '@playwright/test'
import { expect } from '../fixtures'
import type { ControlClient } from './control'

export async function createSessionFromPage(page: Page, title: string) {
  await page.goto('/')
  const newSession = page.getByRole('button', { name: '新建仿真' })
  if (!await newSession.isVisible()) {
    await page.getByRole('button', { name: '展开历史侧栏' }).click()
    await expect(newSession).toBeVisible()
  }
  const responsePromise = page.waitForResponse((response) =>
    response.url().endsWith('/api/sessions') && response.request().method() === 'POST')
  await newSession.click()
  const response = await responsePromise
  expect(response.status()).toBe(201)
  const sessionId = (await response.json()).id as string
  const sessionMenu = page.getByRole('button', { name: '打开“未命名”的会话操作' })
  if (!await sessionMenu.isVisible()) {
    await page.getByRole('button', { name: '展开历史侧栏' }).click()
    await expect(sessionMenu).toBeVisible()
  }
  await sessionMenu.click()
  await page.getByRole('button', { name: '重命名“未命名”' }).click()
  const renameResponse = page.waitForResponse((candidate) =>
    candidate.url().endsWith(`/api/sessions/${sessionId}`) && candidate.request().method() === 'PATCH')
  await page.getByRole('textbox', { name: '重命名“”' }).fill(title)
  await page.getByRole('textbox', { name: '重命名“”' }).press('Enter')
  expect((await renameResponse).ok()).toBeTruthy()
  if (await page.getByRole('button', { name: '关闭历史侧栏' }).isVisible()) {
    await page.getByRole('button', { name: '收起历史侧栏' }).click()
  }
  await expect(page.getByRole('heading', { name: title })).toBeVisible()
  return sessionId
}

export async function createReadySession(page: Page, api: APIRequestContext, title: string) {
  const sessionId = await createSessionFromPage(page, title)
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/bus.csv'))
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.status).toBe('ready')
  const workspace = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  return { sessionId, versionId: workspace.active.versionId as string }
}

export async function send(page: Page, text: string) {
  await page.getByRole('textbox', { name: '消息' }).fill(text)
  await page.getByRole('button', { name: '发送消息' }).click()
}

export async function runStudy(options: {
  page: Page; api: APIRequestContext; control: ControlClient; sessionId: string; versionId: string
  prompt: string; studyType: string; engine: unknown; script?: string
}) {
  const { page, api, control, sessionId, versionId, prompt, studyType, engine } = options
  await control.enqueueEngine(engine)
  await control.enqueueLlm({ id: `study-${prompt}`, expectedPrompt: prompt, turns: [
    { kind: 'tool_call', call_id: `plan-${prompt}`, name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
    { kind: 'tool_call', call_id: `rss-${prompt}`, name: 'load_agent_skill', args: { skill_id: 'rss-generation' } },
    { kind: 'tool_call', call_id: `run-${prompt}`, name: 'run_study', args: { modelVersionId: Number(versionId), studyType, studySpec: { prompt }, script: options.script ?? 'result = 1\n' } },
    { kind: 'text', text: `${prompt}已处理` },
  ] })
  await send(page, prompt)
  await expect(page.getByText(`${prompt}已处理`, { exact: true })).toBeVisible()
  let task: any
  await expect.poll(async () => {
    const tasks = await (await api.get(`/api/sessions/${sessionId}/tasks`)).json()
    task = tasks.find((candidate: any) => candidate.studySpec?.prompt === prompt)
    return task?.status ?? ''
  }).toMatch(/succeeded|failed/)
  return task
}
