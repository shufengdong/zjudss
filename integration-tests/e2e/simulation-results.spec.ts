import { test, expect } from './fixtures'
import { createReadySession, runStudy, send } from './helpers/journeys'

test('natural language runs all study families and exposes reproducible multi-result Parquet data', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'studies@example.com', tenant: 'grid-c' })
  const { sessionId, versionId } = await createReadySession(page, api, '全部仿真类型')
  const longRows = Array.from({ length: 101 }, (_, index) => [`bus-${index}`, index === 100 ? 99 : index])
  longRows[100] = ['bus-99', 99]
  const studies = [
    ['运行潮流', 'power_flow', [{ name: 'voltage', rows: longRows, valueUnit: 'pu' }, { name: 'loss', rows: [['total', 2.5]], valueUnit: 'kW' }]],
    ['运行短路', 'fault', [{ name: 'fault_current', rows: [['bus-1', 8.2]], valueUnit: 'kA' }]],
    ['运行时序', 'qsts', [{ name: 'qsts_voltage', rows: [['t1', 1.01]], valueUnit: 'pu' }]],
    ['运行谐波', 'harmonic', [{ name: 'thd', rows: [['bus-1', 0.03]], valueUnit: 'pu' }]],
    ['运行开放分析', 'open_study', [{ name: 'custom', rows: [['x', 7]], valueUnit: null }]],
  ] as const
  const completed: any[] = []
  for (const [prompt, studyType, data] of studies) {
    completed.push(await runStudy({ page, api, control, sessionId, versionId, prompt, studyType, engine: { kind: 'success', data } }))
  }
  for (const task of completed) {
    expect(task.status).toBe('succeeded')
    expect(task.modelVersionId).toBe(versionId)
    expect(task.scriptArtifactId).not.toBe('')
    expect(task.agentBundleVersion).toBe('v2.0.0')
    expect(task.engineContractVersion).toBe('tensoreval-parquet-v2')
    expect(task.agentSkillRefs.map((item: any) => item.id)).toEqual(['study-planning', 'rss-generation'])
  }
  const datasets = await (await api.get(`/api/tasks/${completed[0].id}/datasets`)).json()
  expect(datasets).toHaveLength(2)
  const voltage = datasets.find((dataset: any) => dataset.name === 'voltage')
  expect(voltage.columns.map((column: any) => column.name)).toEqual(['key', 'value'])
  expect(voltage.columns[1].unit).toBe('pu')
  expect(voltage.rowCount).toBe(101)
  const download = await api.get(`/api/datasets/${voltage.id}/download`)
  expect(download.headers()['content-type']).toBe('application/vnd.apache.parquet')

  await page.getByRole('button', { name: '查看 voltage' }).click()
  await expect(page.getByRole('columnheader', { name: 'value (pu)' })).toBeVisible()
  await expect(page.getByRole('button', { name: '加载更多' })).toBeVisible()
  await page.getByRole('button', { name: '加载更多' }).click()
  await expect(page.getByRole('cell', { name: 'bus-99' })).toHaveCount(2)
  await page.getByText('选择列', { exact: true }).click()
  await page.getByRole('checkbox', { name: '显示列 key' }).uncheck()
  await expect(page.getByRole('columnheader', { name: 'key' })).toHaveCount(0)
  await expect(page.getByRole('columnheader', { name: 'value (pu)' })).toBeVisible()
  const downloadPromise = page.waitForEvent('download')
  await page.getByRole('button', { name: '下载 Parquet' }).click()
  expect((await downloadPromise).suggestedFilename()).toContain('.parquet')
  await page.getByRole('button', { name: '×' }).last().click()

  await control.enqueueEngine({ kind: 'timeout', data: { barrier_id: 'study-timeout' } })
  await control.enqueueLlm({ id: 'failed-study', expectedPrompt: '运行会失败的分析', turns: [
    { kind: 'tool_call', call_id: 'fail-plan', name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
    { kind: 'tool_call', call_id: 'fail-rss', name: 'load_agent_skill', args: { skill_id: 'rss-generation' } },
    { kind: 'tool_call', call_id: 'fail-run', name: 'run_study', args: { modelVersionId: Number(versionId), studyType: 'power_flow', studySpec: { prompt: 'failure' }, script: 'result=1' } },
    { kind: 'text', text: '失败场景已返回' },
  ] })
  await send(page, '运行会失败的分析')
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/tasks`)).json())[0]?.status).toBe('running')
  await control.release('study-timeout')
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/tasks`)).json())[0]?.status).toBe('failed')
  await page.getByRole('button', { name: '仿真历史' }).click()
  await expect(page.getByRole('button', { name: /^open_study #/ })).toBeVisible()
  await expect(page.getByText('failed', { exact: true })).toBeVisible()
})
