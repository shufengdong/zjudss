import { resolve } from 'node:path'
import type { APIRequestContext, Page } from '@playwright/test'
import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'

const uploads = resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes')

async function workspace(api: APIRequestContext, sessionId: string) {
  return (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()) as any
}
const current=(value:any)=>value.draft??value.active

async function upload(page: Page, paths: string[]) {
  await page.locator('input[type=file]').first().setInputFiles(paths)
  await expect(page.getByLabel(/^上传 .*：accepted$/)).toHaveCount(paths.length)
}

test('headers drive CSV Excel and Parquet inference while ambiguous and invalid tables stay collecting', async ({ appPage }) => {
  const { page, api } = await appPage({ subject: 'dataframe-evidence@example.com', tenant: 'grid-b' })
  const sessionId = await createSessionFromPage(page, '表头推断证据')
  await upload(page, [
    resolve(uploads, 'bus.csv'), resolve(uploads, 'bus-duplicate.csv'), resolve(uploads, 'load.csv'),
    resolve(uploads, 'load.parquet'), resolve(uploads, 'ambiguous.csv'), resolve(uploads, 'unknown.csv'),
    resolve(uploads, 'missing.csv'), resolve(uploads, 'dtype-conflict.csv'), resolve(uploads, 'multi.xlsx'),
  ])
  await expect.poll(async () => current(await workspace(api, sessionId))?.status).toBe('collecting')
  await expect(page.getByText('dataframes · collecting', { exact: false })).toBeVisible()
  await expect(page.getByText('待识别', { exact: false }).first()).toBeVisible()
  await expect(page.getByText('缺列：', { exact: false }).first()).toBeVisible()
  await expect(page.getByText('类型冲突：', { exact: false }).first()).toBeVisible()
  const draft = current(await workspace(api, sessionId))
  expect(draft.inputs.some((input: any) => input.fileName === 'bus.csv' && input.resolution === 'pending')).toBe(true)
  expect(draft.inputs.some((input: any) => input.fileName === 'load.parquet')).toBe(true)
  expect(draft.inputs.some((input: any) => input.sheetName === 'Loads')).toBe(true)
  const mixed = draft.inputs.find((input: any) => input.sheetName === 'Mixed')
  expect(mixed, 'Mixed sheet must not be silently dropped').toBeTruthy()
  expect(mixed.resolution).toBe('pending')
  expect(mixed.typeConflicts.length).toBeGreaterThan(0)
  const duplicate = draft.inputs.find((input: any) => input.sheetName === 'DuplicateColumns')
  expect(duplicate, 'DuplicateColumns sheet must not be silently dropped').toBeTruthy()
  expect(duplicate.resolution).toBe('pending')
  expect(duplicate.candidates).toBeInstanceOf(Array)
  expect(draft.inputs.some((input: any) => input.sheetName === 'Empty')).toBe(false)
})

test('supplemental batches create immutable children and natural language confirms and activates mappings', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'dataframe-ready@example.com', tenant: 'grid-b' })
  const sessionId = await createSessionFromPage(page, '分批补传')
  await upload(page, [resolve(uploads, 'bus.csv')])
  await expect.poll(async () => current(await workspace(api, sessionId))?.status).toBe('ready')
  const first = current(await workspace(api, sessionId))
  await upload(page, [resolve(uploads, 'load.parquet')])
  await expect.poll(async () => (await workspace(api, sessionId)).draft?.inputs?.length).toBe(2)
  const second = (await workspace(api, sessionId)).draft
  expect(second.parentVersionId).toBe(first.versionId)

  await control.enqueueLlm({ id: 'activate-dataframes', expectedPrompt: '激活候选', turns: [
    { kind: 'tool_call', call_id: 'activate-df', name: 'activate_model_version', args: { versionId: Number(second.versionId), expectedDraftVersionId: Number(second.versionId) } },
    { kind: 'text', text: '候选模型已激活' },
  ] })
  await page.getByRole('button', { name: '在对话中激活' }).click()
  await expect(page.getByText('候选模型已激活', { exact: true })).toBeVisible()
  await expect.poll(async () => (await workspace(api, sessionId)).active?.versionId).toBe(second.versionId)

  const ambiguousSession = await createSessionFromPage(page, '自然语言确认映射')
  await upload(page, [resolve(uploads, 'bus.csv')])
  await expect.poll(async () => (await workspace(api, ambiguousSession)).active?.status).toBe('ready')
  await upload(page, [resolve(uploads, 'ambiguous.csv')])
  await expect.poll(async () => (await workspace(api, ambiguousSession)).draft?.status).toBe('collecting')
  const ambiguous = (await workspace(api, ambiguousSession)).draft
  const input = ambiguous.inputs.find((candidate: any) => candidate.fileName === 'ambiguous.csv')
  await control.enqueueLlm({ id: 'confirm-mapping', expectedPrompt: '确认这个表是负荷', turns: [
    { kind: 'tool_call', call_id: 'confirm-df', name: 'confirm_dataframe_mapping', args: { inputId: Number(input.inputId), tableKind: 'load', expectedDraftVersionId: Number(ambiguous.versionId) } },
    { kind: 'text', text: '已按负荷表确认' },
  ] })
  await page.getByRole('textbox', { name: '消息' }).fill('确认这个表是负荷')
  await page.getByRole('button', { name: '发送消息' }).click()
  await expect(page.getByText('已按负荷表确认', { exact: true })).toBeVisible()
  await expect.poll(async () => (await workspace(api, ambiguousSession)).draft?.status).toBe('ready')
  const confirmed = (await workspace(api, ambiguousSession)).draft
  await control.enqueueLlm({ id: 'activate-confirmed', expectedPrompt: '激活已确认模型', turns: [
    { kind: 'tool_call', call_id: 'activate-confirmed', name: 'activate_model_version', args: { versionId: Number(confirmed.versionId), expectedDraftVersionId: Number(confirmed.versionId) } },
    { kind: 'text', text: '已激活确认后的模型' },
  ] })
  await page.getByRole('textbox', { name: '消息' }).fill('激活已确认模型')
  await page.getByRole('button', { name: '发送消息' }).click()
  await expect.poll(async () => (await workspace(api, ambiguousSession)).active?.versionId).toBe(confirmed.versionId)
})
