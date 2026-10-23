import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createReadySession, uploadScriptAndRun, send } from './helpers/journeys'
import { restartBackend } from './helpers/server'

test('committed sessions, results, and views survive restart while a running script converges and FTS fails locally', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'restart@example.com', tenant: 'grid-r' })
  const { sessionId } = await createReadySession(page, api, '重启恢复')
  const { run } = await uploadScriptAndRun({
    page, api, control, sessionId,
    engine: { kind: 'success', data: [{ name: 'voltage', rows: [['b1', 1], ['b2', 0.99]], valueUnit: 'pu' }] },
  })
  const source = (await (await api.get(`/api/runs/${run.id}/datasets`)).json()).datasets[0]
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

  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/rss/replacement.rss'))
  const restartCard = page.getByRole('article', { name: /脚本 / }).last()
  await control.enqueueEngine({ kind: 'timeout', data: { barrier_id: 'restart-running' } })
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
