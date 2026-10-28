import { resolve } from 'node:path'
import type { APIRequestContext, Locator, Page } from '@playwright/test'
import { expect } from '../fixtures'
import type { ControlClient } from './control'
import { send } from './journeys'
import { confirmLatestModelUpload } from './uploads'

const fixtures = resolve(process.cwd(), '../integration-tests/fixtures/uploads')

export const newtonScript = `[bus_voltage] = ds_newton_raphson_pf(
    bus, line, linecode, load, transformer, voltage_source,
    motor, transformer_winding, transformer_impedance, current_source,
    generator, ibr, switch, shunt, capacitor, reactor, spectrum,
    control_profile, xy_curve, storage, para
);`

export async function settleNewtonConfiguration(page: Page, configuration: Locator) {
  const confirmConfiguration = configuration.getByRole('button', { name: '确认并生成脚本' })
  const confirmParameters = configuration.getByRole('button', { name: '确认参数' })
  await expect.poll(async () => await confirmConfiguration.isVisible() || await confirmParameters.isVisible()).toBe(true)
  if (!await confirmConfiguration.isVisible()) {
    const frequency = configuration.getByLabel(/base_frequency_hz/)
    if (await frequency.isVisible() && await frequency.inputValue() === '') await frequency.fill('50')
    const response = page.waitForResponse(candidate => candidate.request().method() === 'PUT'
      && /\/api\/study-configurations\/\d+\/structs\//.test(new URL(candidate.url()).pathname))
    await confirmParameters.click()
    expect((await response).ok()).toBe(true)
  }
  await expect(confirmConfiguration).toBeVisible()
}

export async function publishExampleModel(page: Page, api: APIRequestContext, sessionId: string) {
  await page.locator('input[type=file]').nth(1).setInputFiles(resolve(fixtures, 'bundle/demo.rssbundle'))
  await confirmLatestModelUpload(page)
  await expect.poll(async () => ((await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()) as any).draft?.tables?.length).toBe(20)
  if (await page.getByRole('button', { name: '展开结果面板' }).isVisible()) {
    await page.getByRole('button', { name: '展开结果面板' }).click()
  }
  await page.getByRole('button', { name: /^展开可变草稿/ }).click()
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => ((await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()) as any).active?.id).toBeTruthy()
}

export async function prepareNewtonStudy(page: Page, api: APIRequestContext, sessionId: string, control: ControlClient) {
  await control.enqueueLlm({
    id: `newton-${Date.now()}-${Math.random()}`,
    expectedPrompt: '求解牛顿法潮流',
    turns: [
      { kind: 'tool_call', call_id: 'inspect', name: 'inspect_workspace', args: {} },
      { kind: 'tool_call', call_id: 'skill', name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
      { kind: 'tool_call', call_id: 'search', name: 'search_study_facades', args: { query: '牛顿法潮流', explicitFunction: 'ds_newton_raphson_pf' } },
      { kind: 'tool_call', call_id: 'prepare', name: 'prepare_study', args: { facadeId: 'static_power_flow_newton', intent: { request: '求解牛顿法潮流' } } },
      { kind: 'text', text: '研究配置已准备，请确认后开始。' },
    ],
  })
  await send(page, '求解牛顿法潮流')
  await expect(page.getByText('研究配置已准备，请确认后开始。').last()).toBeVisible()
  const configuration = page.getByRole('article', { name: '研究配置卡' }).last()
  const snapshot = await (await api.get(`/api/sessions/${sessionId}/messages`)).json() as { messages: Array<{ type: string; card?: { configurationId?: string } }> }
  await expect(configuration, JSON.stringify(snapshot, null, 2)).toContainText('牛顿-拉夫逊潮流')
  await settleNewtonConfiguration(page, configuration)
  const message = [...snapshot.messages].reverse().find(item => item.type === 'study-config' && item.card?.configurationId)
  expect(message?.card?.configurationId).toBeTruthy()
  return { card: configuration, id: message!.card!.configurationId! }
}
