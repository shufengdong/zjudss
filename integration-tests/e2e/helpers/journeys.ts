import { resolve } from 'node:path'
import type { APIRequestContext, Page } from '@playwright/test'
import { expect } from '../fixtures'
import type { ControlClient } from './control'

export async function createSessionFromPage(page: Page, title: string) {
  await page.goto('/')
  await expect.poll(async () => {
    const response = await page.request.get('/api/sessions')
    return ((await response.json()) as { sessions: unknown[] }).sessions.length
  }).toBeGreaterThan(0)
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
  const sessionMenu = page.getByRole('button', { name: '打开“新会话”的会话操作' }).first()
  if (!await sessionMenu.isVisible()) {
    await page.getByRole('button', { name: '展开历史侧栏' }).click()
    await expect(sessionMenu).toBeVisible()
  }
  await sessionMenu.click()
  await page.getByRole('button', { name: '重命名“新会话”' }).click()
  const renameResponse = page.waitForResponse((candidate) =>
    candidate.url().endsWith(`/api/sessions/${sessionId}`) && candidate.request().method() === 'PATCH')
  await page.getByRole('textbox', { name: '重命名“新会话”' }).fill(title)
  await page.getByRole('textbox', { name: '重命名“新会话”' }).press('Enter')
  expect((await renameResponse).ok()).toBeTruthy()
  if (await page.getByRole('button', { name: '关闭历史侧栏' }).isVisible()) {
    await page.getByRole('button', { name: '收起历史侧栏' }).click()
  }
  await expect(page.getByRole('heading', { name: title, level: 1 })).toBeVisible()
  return sessionId
}

export async function createReadySession(page: Page, api: APIRequestContext, title: string) {
  const sessionId = await createSessionFromPage(page, title)
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/bus.csv'))
  if (!await page.getByRole('button', { name: '确认合并' }).isVisible() && await page.getByRole('button', { name: '展开结果面板' }).isVisible()) {
    await page.getByRole('button', { name: '展开结果面板' }).click()
  }
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()
  await page.getByRole('button', { name: '确认合并' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.revision).toBe(1)
  await page.getByRole('button', { name: /^展开可变草稿/ }).click()
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.id).toBeTruthy()
  const workspace = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  if (await page.getByRole('button', { name: '展开历史侧栏' }).isVisible() && await page.getByRole('button', { name: '关闭结果面板' }).isVisible()) {
    await page.getByRole('button', { name: '关闭结果面板' }).click()
  }
  return { sessionId, versionId: workspace.active.id as string }
}

export async function send(page: Page, text: string) {
  await page.getByRole('textbox', { name: '消息' }).fill(text)
  await page.getByRole('button', { name: '发送消息' }).click()
}

export async function uploadScriptAndRun(options: {
  page: Page; api: APIRequestContext; control: ControlClient; sessionId: string; engine: unknown; file?: string
}) {
  const { page, api, control, sessionId, engine } = options
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), options.file ?? '../integration-tests/fixtures/uploads/rss/replacement.rss'))
  const card = page.getByRole('article', { name: /脚本 / }).last()
  await expect(card).toBeVisible()
  expect((await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs).toHaveLength(0)
  await control.enqueueEngine(engine)
  await card.getByRole('button', { name: '运行' }).click()
  let run: any
  await expect.poll(async () => {
    const runs = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs
    run = runs[0]
    return run?.status ?? ''
  }).toMatch(/succeeded|failed/)
  return { run, card }
}
