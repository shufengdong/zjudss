import { test, expect } from './fixtures'
import { createReadySession, runStudy, send } from './helpers/journeys'

test('result analysis creates traceable Parquet and all six controlled ECharts views survive reload', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'analysis@example.com', tenant: 'grid-c' })
  const { sessionId, versionId } = await createReadySession(page, api, '结果分析与图表')
  const task = await runStudy({ page, api, control, sessionId, versionId, prompt: '先运行基础潮流', studyType: 'power_flow', engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1], ['b2', 1.04], ['b3', 0.98]], valueUnit: 'pu' }] } })
  const source = (await (await api.get(`/api/tasks/${task.id}/datasets`)).json())[0]
  const viewSpecs = [
    ['折线', { kind: 'line', x: 'key', y: ['value'] }],
    ['柱状', { kind: 'bar', x: 'key', y: ['value'] }],
    ['散点', { kind: 'scatter', x: 'value', y: ['value'] }],
    ['热力', { kind: 'heatmap', x: 'key', category: 'key', value: 'value' }],
    ['箱线', { kind: 'boxplot', category: 'key', value: 'value' }],
    ['直方图', { kind: 'histogram', value: 'value', bins: 2 }],
  ]
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'derived', rows: [['b1', 1], ['b2', 1.04], ['b3', 0.98]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'analysis-six-views', expectedPrompt: '分析电压并生成六种图', turns: [
    { kind: 'tool_call', call_id: 'list-schema', name: 'list_result_datasets', args: { taskId: Number(task.id) } },
    { kind: 'tool_call', call_id: 'analysis-skill', name: 'load_agent_skill', args: { skill_id: 'result-analysis' } },
    { kind: 'tool_call', call_id: 'run-analysis', name: 'run_result_analysis', args: {
      intent: { goal: 'six views' }, script: 'derived = source', inputs: [{ datasetId: Number(source.id), alias: 'source' }], expectedOutputs: ['derived'],
      initialViews: { derived: viewSpecs.map(([name, spec]) => ({ name, spec })) },
    } },
    { kind: 'text', text: '分析和六种视图已生成' },
  ] })
  await send(page, '分析电压并生成六种图')
  await expect(page.getByText('分析和六种视图已生成', { exact: true })).toBeVisible()
  const analysisResult = page.getByTestId('message-result').last()
  await expect(analysisResult.getByRole('button', { name: '查看 derived' })).toBeVisible()
  const analyses = await (await api.get(`/api/sessions/${sessionId}/messages`)).json()
  const resultMessage = analyses.messages.find((message: any) => message.type === 'result' && message.analysisRunId)
  const analysis = await (await api.get(`/api/analysis-runs/${resultMessage.analysisRunId}`)).json()
  expect(analysis.status).toBe('succeeded')
  expect(analysis.agentSkillRefs[0].id).toBe('result-analysis')
  expect(analysis.datasets[0].analysisRunId).toBe(analysis.id)
  expect(analysis.datasets[0].views).toHaveLength(6)

  await analysisResult.getByRole('button', { name: '查看 derived' }).click()
  for (const [index, [name]] of viewSpecs.entries()) {
    await page.getByRole('button', { name: String(name), exact: true }).click()
    const kind = ['line', 'bar', 'scatter', 'heatmap', 'boxplot', 'histogram'][index]
    await expect(page.getByLabel(`${kind} 图表`)).toBeVisible()
  }
  await page.getByRole('button', { name: '关闭数据集' }).click()

  await control.enqueueLlm({ id: 'invalid-view', expectedPrompt: '创建一个非法字段图', turns: [
    { kind: 'tool_call', call_id: 'bad-view', name: 'create_dataset_view', args: { datasetId: Number(analysis.datasets[0].id), name: '非法', spec: { kind: 'line', x: 'missing', y: ['value'] } } },
    { kind: 'text', text: '非法视图已被拒绝' },
  ] })
  await send(page, '创建一个非法字段图')
  await expect(page.getByText('非法视图已被拒绝', { exact: true })).toBeVisible()
  await expect(page.getByText('create_dataset_view', { exact: true }).last()).toBeVisible()
  await page.reload()
  await page.getByRole('button', { name: '查看 derived' }).click()
  await expect(page.getByRole('button', { name: '直方图', exact: true })).toBeVisible()
})
