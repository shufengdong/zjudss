import { test, expect } from './fixtures'
import { createReadySession, runStudy, send } from './helpers/journeys'

test('compatible tasks create a recoverable difference dataset while name and unit mismatches fail closed', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'compare@example.com', tenant: 'grid-c' })
  const { sessionId, versionId } = await createReadySession(page, api, '仿真比较')
  const baseline = await runStudy({ page, api, control, sessionId, versionId, prompt: '运行基准潮流', studyType: 'power_flow', engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] } })
  const candidate = await runStudy({ page, api, control, sessionId, versionId, prompt: '运行候选潮流', studyType: 'power_flow', engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1.02], ['b2', 1]], valueUnit: 'pu' }] } })
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'result', rows: [['b1', 0.02], ['b2', 0.01]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'compare-compatible', expectedPrompt: '比较基准和候选', turns: [
    { kind: 'tool_call', call_id: 'comparison-skill', name: 'load_agent_skill', args: { skill_id: 'comparison' } },
    { kind: 'tool_call', call_id: 'compare', name: 'compare_simulations', args: { baselineTaskId: Number(baseline.id), candidateTaskId: Number(candidate.id) } },
    { kind: 'text', text: '可比任务差异已生成' },
  ] })
  await send(page, '比较基准和候选')
  await expect(page.getByText('可比任务差异已生成', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '查看比较' }).click()
  await expect(page.getByText(`任务 ${baseline.id} 与 ${candidate.id}`, { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'result', exact: true }).click()
  await expect(page.getByRole('columnheader', { name: 'value (pu)' })).toBeVisible()
  await page.getByRole('button', { name: '关闭数据集' }).click()
  await page.reload()
  await page.getByRole('button', { name: '查看比较' }).click()
  await expect(page.getByText(`任务 ${baseline.id} 与 ${candidate.id}`, { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '关闭比较' }).click()

  const wrongUnit = await runStudy({ page, api, control, sessionId, versionId, prompt: '运行不同单位潮流', studyType: 'power_flow', engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 10]], valueUnit: 'kV' }] } })
  const wrongName = await runStudy({ page, api, control, sessionId, versionId, prompt: '运行不同结果名潮流', studyType: 'power_flow', engine: { kind: 'success', data: [{ name: 'current', rows: [['b1', 1]], valueUnit: 'pu' }] } })
  for (const [prompt, other] of [['比较不同单位', wrongUnit], ['比较不同结果名', wrongName]] as const) {
    await control.enqueueLlm({ id: prompt, expectedPrompt: prompt, turns: [
      { kind: 'tool_call', call_id: `${prompt}-skill`, name: 'load_agent_skill', args: { skill_id: 'comparison' } },
      { kind: 'tool_call', call_id: `${prompt}-tool`, name: 'compare_simulations', args: { baselineTaskId: Number(baseline.id), candidateTaskId: Number(other.id) } },
      { kind: 'text', text: `${prompt}已拒绝` },
    ] })
    await send(page, prompt)
    await expect(page.getByText(`${prompt}已拒绝`, { exact: true })).toBeVisible()
  }
  const messages = await (await api.get(`/api/sessions/${sessionId}/messages?limit=100`)).json()
  expect(messages.messages.filter((message: any) => message.type === 'comparison')).toHaveLength(1)
})
