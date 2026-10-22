import { test, expect } from './fixtures'
import { createReadySession, runStudy, send } from './helpers/journeys'
import { restartBackend } from './helpers/server'

test('committed sessions results and views survive restart while running work converges and FTS fails locally', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'restart@example.com', tenant: 'grid-r' })
  const { sessionId, versionId } = await createReadySession(page, api, '重启恢复')
  const task = await runStudy({
    page, api, control, sessionId, versionId, prompt: '运行重启前潮流', studyType: 'power_flow',
    engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] },
  })
  const source = (await (await api.get(`/api/tasks/${task.id}/datasets`)).json())[0]
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'derived', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'persisted-analysis', expectedPrompt: '创建可恢复视图', turns: [
    { kind: 'tool_call', call_id: 'analysis-skill', name: 'load_agent_skill', args: { skill_id: 'result-analysis' } },
    { kind: 'tool_call', call_id: 'analysis-run', name: 'run_result_analysis', args: {
      intent: { goal: 'restart recovery' }, script: 'derived = source', inputs: [{ datasetId: Number(source.id), alias: 'source' }], expectedOutputs: ['derived'],
      initialViews: { derived: [{ name: '恢复折线', spec: { kind: 'line', x: 'key', y: ['value'] } }] },
    } },
    { kind: 'text', text: '持久分析完成' },
  ] })
  await send(page, '创建可恢复视图')
  await expect(page.getByText('持久分析完成', { exact: true })).toBeVisible()

  await control.enqueueEngine({ kind: 'timeout', data: { barrier_id: 'restart-running' } })
  await control.enqueueLlm({ id: 'restart-running', expectedPrompt: '运行会被重启中断的仿真', turns: [
    { kind: 'tool_call', call_id: 'restart-plan', name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
    { kind: 'tool_call', call_id: 'restart-rss', name: 'load_agent_skill', args: { skill_id: 'rss-generation' } },
    { kind: 'tool_call', call_id: 'restart-run', name: 'run_study', args: { modelVersionId: Number(versionId), studyType: 'power_flow', studySpec: { prompt: 'restart-running' }, script: 'result=1' } },
  ] })
  await send(page, '运行会被重启中断的仿真')
  let interrupted: any
  await expect.poll(async () => {
    const tasks = await (await api.get(`/api/sessions/${sessionId}/tasks`)).json()
    interrupted = tasks.find((candidate: any) => candidate.studySpec?.prompt === 'restart-running')
    return interrupted?.status
  }).toBe('running')

  await restartBackend()
  await expect.poll(async () => (await (await api.get(`/api/tasks/${interrupted.id}`)).json()).status).toBe('failed')
  const recovered = await (await api.get(`/api/tasks/${interrupted.id}`)).json()
  expect(recovered.stage).toBe('recovery')
  expect(recovered.error.code).toBe('service_restarted')

  await page.reload()
  await expect(page.getByText('持久分析完成', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '查看 derived' }).click()
  await page.getByRole('button', { name: '恢复折线', exact: true }).click()
  await expect(page.getByLabel('line 图表')).toBeVisible()
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
