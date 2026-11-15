import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createReadySession, uploadScriptAndRun, send } from './helpers/journeys'
import { restartBackend } from './helpers/server'
import { confirmLatestUpload, openMutableDraft } from './helpers/uploads'

test('analysis details and committed results survive restart while cross-version input is rejected', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'restart@example.com', tenant: 'grid-r' })
  const { sessionId, versionId } = await createReadySession(page, api, '重启恢复')
  const workspace = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  const bus = workspace.active.tables.find((table: any) => table.tableName === 'bus')
  expect(bus?.id).toBeTruthy()
  const { run } = await uploadScriptAndRun({
    page, api, control, sessionId,
    engine: { kind: 'success', data: [{ name: 'bus_voltage', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] },
  })
  const source = (await (await api.get(`/api/runs/${run.id}/datasets`)).json()).datasets[0]
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'derived', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'persisted-analysis', expectedPrompt: '筛选并保存可恢复的派生结果', turns: [
    { kind: 'tool_call', call_id: 'analysis-skill', name: 'load_agent_skill', args: { skill_id: 'result-analysis' } },
    { kind: 'tool_call', call_id: 'analysis-run', name: 'run_result_analysis', args: {
      intent: 'use the active bus model and matching bus voltage result to persist a recoverable derived dataframe',
      script: 'derived = limit(2, bus_voltage);',
      inputs: [
        { modelDataframeId: Number(bus.id), alias: 'bus' },
        { datasetId: Number(source.id), alias: 'bus_voltage' },
      ],
      expectedOutputs: ['derived'],
    } },
    { kind: 'text', text: '持久分析完成' },
  ] })
  await send(page, '筛选并保存可恢复的派生结果')
  await expect(page.getByText('持久分析完成', { exact: true })).toBeVisible()

  const analysisMessages = (await (await api.get(`/api/sessions/${sessionId}/messages?limit=100`)).json()).messages
  const analysisMessage = analysisMessages.find((message: any) => message.type === 'result' && message.analysisRunId)
  expect(analysisMessage?.analysisRunId).toBeTruthy()
  const analysis = await (await api.get(`/api/analysis-runs/${analysisMessage.analysisRunId}`)).json()
  expect(analysis.inputs.map((input: any) => input.sourceKind)).toEqual(['active_model', 'dataset'])
  expect(analysis.inputs[0].modelVersionId).toBe(versionId)
  expect(analysis.inputs[1].datasetId).toBe(source.id)
  await page.getByRole('button', { name: '查看 derived' }).click()
  await expect(page.getByRole('table')).toBeVisible()
  await page.getByRole('button', { name: '关闭数据集' }).click()
  await page.getByRole('button', { name: '查看分析脚本与输入' }).click()
  await expect(page.getByLabel('分析 RSS 脚本')).toContainText('derived = limit(2, bus_voltage);')
  await expect(page.getByTestId('analysis-input')).toHaveCount(2)

  await page.reload()
  await expect(page.getByText('持久分析完成', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '查看分析脚本与输入' }).click()
  await expect(page.getByLabel('分析 RSS 脚本')).toContainText('derived = limit(2, bus_voltage);')
  await expect(page.getByTestId('analysis-input')).toHaveCount(2)

  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.csv'))
  await confirmLatestUpload(page, 'model_table', { 'load.csv': 'load' })
  await openMutableDraft(page)
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.id).not.toBe(versionId)
  await control.enqueueLlm({ id: 'mismatched-analysis', expectedPrompt: '尝试分析旧版本结果', turns: [
    { kind: 'tool_call', call_id: 'mismatch-skill', name: 'load_agent_skill', args: { skill_id: 'result-analysis' } },
    { kind: 'tool_call', call_id: 'mismatch-run', name: 'run_result_analysis', args: {
      intent: 'derive from a result that belongs to a different model version',
      script: 'should_not_exist = limit(1, old_voltage);',
      inputs: [{ datasetId: Number(source.id), alias: 'old_voltage' }],
      expectedOutputs: ['should_not_exist'],
    } },
    { kind: 'text', text: '旧版本结果与当前模型不匹配，未创建分析运行' },
  ] })
  await send(page, '尝试分析旧版本结果')
  await expect(page.getByText('旧版本结果与当前模型不匹配，未创建分析运行', { exact: true })).toBeVisible()
  const afterMismatch = (await (await api.get(`/api/sessions/${sessionId}/messages?limit=100`)).json()).messages
  expect(afterMismatch.filter((message: any) => message.type === 'result' && message.analysisRunId)).toHaveLength(1)
  expect(afterMismatch.some((message: any) => message.type === 'tool-call'
    && message.toolName === 'run_result_analysis' && message.status === 'failed')).toBe(true)

  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/rss/replacement.rss'))
  const restartCard = page.getByRole('article', { name: /脚本 / }).last()
  await control.enqueueEngine({ kind: 'cancelled', data: { barrier_id: 'restart-running' } })
  await restartCard.getByRole('button', { name: '运行' }).click()
  let interrupted: any
  await expect.poll(async () => {
    const runs = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs
    interrupted = runs.find((candidate: any) => candidate.id !== run.id)
    return interrupted?.status
  }).toBe('running')

  await restartBackend()
  await expect.poll(async () => (await (await api.get(`/api/runs/${interrupted.id}`)).json()).status).toBe('failed')
  const recovered = await (await api.get(`/api/runs/${interrupted.id}`)).json()
  expect(recovered.stage).toBe('recovery')
  expect(recovered.error.code).toBe('service_restarted')

  await page.reload()
  await expect(page.getByText('持久分析完成', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '查看 derived' }).click()
  await expect(page.getByRole('table')).toBeVisible()
  await page.getByRole('button', { name: '关闭数据集' }).click()

  await control.dropHistoryIndex()
  const search = await api.get('/api/history/search?query=%E9%87%8D%E5%90%AF%E6%81%A2%E5%A4%8D')
  expect(search.status()).toBe(503)
  expect((await search.json()).code).toBe('history_search_unavailable')
  const messages = await api.get(`/api/sessions/${sessionId}/messages`)
  expect(messages.status()).toBe(200)
  expect((await messages.json()).messages.length).toBeGreaterThan(0)
  await control.rebuildHistoryIndex()
})
