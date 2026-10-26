import { resolve } from 'node:path'
import { test, expect } from './fixtures'
import { createReadySession, send, uploadScriptAndRun } from './helpers/journeys'

test('model import stays a draft until publish and a script runs only after its explicit button', async ({ appPage, control }) => {
  const { page, api, context } = await appPage({ subject: 'studies@example.com' })
  const { sessionId, versionId } = await createReadySession(page, api, 'V3 显式运行')
  const workspace = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
  expect(workspace.draft).toBeNull()
  expect(workspace.active.id).toBe(versionId)

  const longScript = Array.from({ length: 28 }, (_, index) => `# filler ${index + 1}`).join('\n') + '\nresult = 1\n'
  await page.locator('input[type=file]').first().setInputFiles({
    name: 'long-script.rss', mimeType: 'text/plain', buffer: Buffer.from(longScript),
  })
  const card = page.getByRole('article', { name: '脚本 long-script.rss' })
  await expect(card.getByRole('button', { name: '展开全文' })).toBeVisible()
  await card.getByRole('button', { name: '展开全文' }).click()
  await expect(card.getByText('# filler 28', { exact: false })).toBeVisible()
  await card.getByRole('button', { name: '收起' }).click()
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await card.getByRole('button', { name: '复制' }).click()
  await expect(page.getByText('脚本全文已复制', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(longScript)
  expect((await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs).toHaveLength(0)

  const manualResponse = await api.post(`/api/sessions/${sessionId}/scripts/text`, {
    data: { name: 'manual-script.rss', content: 'result = 1\n' },
  })
  expect(manualResponse.status()).toBe(201)
  const scripts = (await (await api.get(`/api/sessions/${sessionId}/scripts`)).json()).scripts
  expect(scripts.map((script: any) => script.name)).toContain('manual-script.rss')

  await control.enqueueEngine({ kind: 'success', data: [
    { name: 'voltage', rows: [['bus-1', 1.01], ['bus-2', 0.99]], valueUnit: 'pu' },
    { name: 'loss', rows: [['total', 2.5]], valueUnit: 'kW' },
  ] })
  await card.getByRole('button', { name: '运行' }).click()
  let run: any
  await expect.poll(async () => {
    run = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs[0]
    return run?.status
  }).toBe('succeeded')
  expect(run.modelVersionId).toBe(versionId)
  const datasets = (await (await api.get(`/api/runs/${run.id}/datasets`)).json()).datasets
  expect(datasets.map((dataset: any) => dataset.name).sort()).toEqual(['loss', 'voltage'])

  await page.getByRole('button', { name: '运行记录' }).click()
  const runHeader = page.getByRole('button', { name: /long-script\.rss.*成功/ })
  await expect(runHeader).toHaveAttribute('aria-expanded', 'false')
  await runHeader.click()
  const runCard = runHeader.locator('xpath=..')
  await expect(runCard.getByText('voltage', { exact: true })).toBeVisible()
  await expect(runCard.getByText('loss', { exact: true })).toBeVisible()
  await runCard.getByRole('button', { name: '查看 loss' }).click()
  await expect(page.getByRole('heading', { name: 'loss' })).toBeVisible()
  const datasetDialog = page.getByRole('button', { name: '关闭数据集' }).locator('xpath=../../..')
  const dialogBox = await datasetDialog.boundingBox()
  const viewport = page.viewportSize()
  expect(dialogBox).not.toBeNull()
  expect(viewport).not.toBeNull()
  expect(Math.abs(dialogBox!.x + dialogBox!.width / 2 - viewport!.width / 2)).toBeLessThanOrEqual(2)
})

test('one result can be deleted without deleting its run or sibling result', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'analysis@example.com' })
  const { sessionId } = await createReadySession(page, api, '结果删除')
  const { run } = await uploadScriptAndRun({ page, api, control, sessionId, engine: {
    kind: 'success', data: [
      { name: 'keep', rows: [['a', 1]], valueUnit: null },
      { name: 'remove', rows: [['b', 2]], valueUnit: null },
    ],
  } })
  const before = (await (await api.get(`/api/runs/${run.id}/datasets`)).json()).datasets
  const removed = before.find((dataset: any) => dataset.name === 'remove')
  expect((await api.get(`/api/datasets/${removed.id}`)).status()).toBe(200)
  expect((await api.get(`/api/datasets/${removed.id}/rows`)).status()).toBe(200)
  expect((await api.get(`/api/datasets/${removed.id}/download`)).status()).toBe(200)
  expect((await api.delete(`/api/datasets/${removed.id}`)).status()).toBe(200)
  expect((await api.get(`/api/datasets/${removed.id}`)).status()).toBe(404)
  expect((await api.get(`/api/datasets/${removed.id}/rows`)).status()).toBe(404)
  expect((await api.get(`/api/datasets/${removed.id}/download`)).status()).toBe(404)
  expect((await api.get(`/api/runs/${run.id}`)).status()).toBe(200)
  const after = (await (await api.get(`/api/runs/${run.id}/datasets`)).json()).datasets
  expect(after.map((dataset: any) => dataset.name)).toEqual(['keep'])
})

test('agent-created script is visible but never auto-runs; the user click starts it', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'compare@example.com' })
  const { sessionId } = await createReadySession(page, api, 'Agent 生成脚本')
  await control.enqueueLlm({ id: 'create-v3-script', expectedPrompt: '生成一个潮流脚本', turns: [
    { kind: 'tool_call', call_id: 'skill', name: 'load_agent_skill', args: { skill_id: 'rss-generation' } },
    { kind: 'tool_call', call_id: 'create', name: 'create_script', args: { name: 'agent-flow.rss', content: 'result = 1\n' } },
    { kind: 'text', text: '脚本已生成，请点击运行' },
  ] })
  await send(page, '生成一个潮流脚本')
  await expect(page.getByText('脚本已生成，请点击运行', { exact: true })).toBeVisible()
  expect((await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs).toHaveLength(0)
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'result', rows: [['ok', 1]], valueUnit: null }] })
  await page.getByRole('article', { name: '脚本 agent-flow.rss' }).getByRole('button', { name: '运行' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs[0]?.status).toBe('succeeded')
})

test('failed run history shows sanitized tensoreval output without requesting datasets', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'studies@example.com' })
  const { sessionId } = await createReadySession(page, api, '失败运行详情')
  const datasetRequests: string[] = []
  page.on('request', request => {
    if (/\/api\/runs\/[^/]+\/datasets$/.test(new URL(request.url()).pathname)) datasetRequests.push(request.url())
  })
  const { run } = await uploadScriptAndRun({
    page, api, control, sessionId, engine: { kind: 'run_failure' },
  })
  expect(run.status).toBe('failed')
  await page.getByRole('button', { name: '运行记录' }).click()
  const runHeader = page.getByRole('button', { name: /replacement\.rss.*失败/ })
  await runHeader.click()
  const detail = runHeader.locator('xpath=..').getByRole('alert')
  await expect(detail).toContainText('失败阶段：执行计算')
  await expect(detail).toContainText('失败原因：仿真引擎执行失败')
  await expect(detail).toContainText('tensoreval 报错：')
  await expect(detail).toContainText('RSS parse error: unexpected token')
  await expect(detail).toContainText('退出码：7')
  await expect(detail).not.toContainText('/srv/private')
  expect(datasetRequests).toEqual([])
})

test('imports, scripts, runs, results, and deletion remain isolated between users', async ({ loginAs, control }) => {
  const alice = await loginAs('alice@example.com')
  const bob = await loginAs('bob@example.com')
  const { sessionId } = await createReadySession(alice.page, alice.api, 'Alice V3')
  const { run } = await uploadScriptAndRun({ page: alice.page, api: alice.api, control, sessionId, engine: {
    kind: 'success', data: [{ name: 'private_result', rows: [['x', 3]], valueUnit: null }],
  } })
  const dataset = (await (await alice.api.get(`/api/runs/${run.id}/datasets`)).json()).datasets[0]
  for (const path of [
    `/api/sessions/${sessionId}/workspace`, `/api/sessions/${sessionId}/versions`,
    `/api/sessions/${sessionId}/scripts`, `/api/sessions/${sessionId}/runs`,
    `/api/runs/${run.id}`, `/api/runs/${run.id}/datasets`, `/api/datasets/${dataset.id}`,
  ]) expect((await bob.api.get(path)).status(), path).toBe(404)
  expect((await bob.api.delete(`/api/datasets/${dataset.id}`)).status()).toBe(404)
  expect((await alice.api.get(`/api/datasets/${dataset.id}`)).status()).toBe(200)
})

test('analysis, materialized views, and comparisons survive source-result deletion with tombstone lineage', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'dataframe-evidence@example.com' })
  const { sessionId } = await createReadySession(page, api, '派生结果血缘')
  const { run: baseline, card } = await uploadScriptAndRun({ page, api, control, sessionId, engine: {
    kind: 'success', data: [{ name: 'result', rows: [['x', 1]], valueUnit: 'pu' }],
  } })
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'result', rows: [['x', 1.1]], valueUnit: 'pu' }] })
  await card.getByRole('button', { name: '运行' }).click()
  await expect.poll(async () => {
    const runs = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs
    return runs.length === 2 && runs.every((run: any) => run.status === 'succeeded')
  }).toBe(true)
  const runs = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs
  const candidate = runs.find((run: any) => run.id !== baseline.id)
  const source = (await (await api.get(`/api/runs/${baseline.id}/datasets`)).json()).datasets[0]

  await control.enqueueEngine({ kind: 'success', data: [{ name: 'derived', rows: [['x', 1]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'v3-analysis', expectedPrompt: '分析并物化结果', turns: [
    { kind: 'tool_call', call_id: 'analysis-skill', name: 'load_agent_skill', args: { skill_id: 'result-analysis' } },
    { kind: 'tool_call', call_id: 'analysis', name: 'run_result_analysis', args: {
      intent: { goal: 'derive' }, script: 'derived = source', inputs: [{ datasetId: Number(source.id), alias: 'source' }], expectedOutputs: ['derived'],
    } },
    { kind: 'text', text: '派生结果已保存' },
  ] })
  await send(page, '分析并物化结果')
  let analysisMessage: any
  await expect.poll(async () => {
    const messages = (await (await api.get(`/api/sessions/${sessionId}/messages?limit=100`)).json()).messages
    analysisMessage = messages.find((message: any) => message.type === 'result' && message.analysisRunId)
    return analysisMessage?.analysisRunId
  }).toBeTruthy()
  const derivedId = analysisMessage.outputs[0].datasetId
  expect((await api.get(`/api/analysis-runs/${analysisMessage.analysisRunId}`)).status()).toBe(200)

  await control.enqueueLlm({ id: 'v3-view', expectedPrompt: '创建持久折线视图', turns: [
    { kind: 'tool_call', call_id: 'view', name: 'create_dataset_view', args: {
      datasetId: Number(derivedId), name: '持久折线', spec: { kind: 'line', x: 'key', y: ['value'] },
    } },
    { kind: 'text', text: '视图已保存' },
  ] })
  await send(page, '创建持久折线视图')
  await expect(page.getByText('视图已保存', { exact: true })).toBeVisible()
  const derived = await (await api.get(`/api/datasets/${derivedId}`)).json()
  const viewId = derived.views[0].id
  expect((await api.get(`/api/dataset-views/${viewId}/data`)).status()).toBe(200)

  await control.enqueueEngine({ kind: 'success', data: [{ name: 'result', rows: [['x', 0.1]], valueUnit: 'pu' }] })
  await control.enqueueLlm({ id: 'v3-comparison', expectedPrompt: '比较两次运行', turns: [
    { kind: 'tool_call', call_id: 'comparison-skill', name: 'load_agent_skill', args: { skill_id: 'comparison' } },
    { kind: 'tool_call', call_id: 'comparison', name: 'compare_simulations', args: {
      baselineRunId: Number(baseline.id), candidateRunId: Number(candidate.id),
    } },
    { kind: 'text', text: '运行比较已保存' },
  ] })
  await send(page, '比较两次运行')
  let comparisonMessage: any
  await expect.poll(async () => {
    const messages = (await (await api.get(`/api/sessions/${sessionId}/messages?limit=100`)).json()).messages
    comparisonMessage = messages.find((message: any) => message.type === 'comparison')
    return comparisonMessage?.comparisonId
  }).toBeTruthy()
  expect((await api.get(`/api/run-comparisons/${comparisonMessage.comparisonId}`)).status()).toBe(200)

  expect((await api.delete(`/api/datasets/${source.id}`)).status()).toBe(200)
  expect((await api.get(`/api/datasets/${source.id}/rows`)).status()).toBe(404)
  expect((await api.get(`/api/dataset-views/${viewId}/data`)).status()).toBe(200)
  expect((await api.get(`/api/run-comparisons/${comparisonMessage.comparisonId}`)).status()).toBe(200)
  const lineage = await (await api.get(`/api/datasets/${derivedId}`)).json()
  expect(lineage.lineage[0].deleted).toBe(true)
  expect(lineage.lineage[0].tombstone.datasetId).toBe(source.id)
})

test('CSV imports merge by whole table, stale runs are rejected, and history activation/deletion is explicit', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'dataframe-ready@example.com' })
  const { sessionId, versionId: firstVersion } = await createReadySession(page, api, '版本历史')
  const { run: firstRun } = await uploadScriptAndRun({ page, api, control, sessionId, engine: {
    kind: 'success', data: [{ name: 'result', rows: [['x', 1]], valueUnit: null }],
  } })
  const firstDataset = (await (await api.get(`/api/runs/${firstRun.id}/datasets`)).json()).datasets[0]
  const script = (await (await api.get(`/api/sessions/${sessionId}/scripts`)).json()).scripts[0]
  expect((await api.get(`/api/scripts/${script.id}`)).status()).toBe(200)
  expect((await api.get(`/api/versions/${firstVersion}`)).status()).toBe(200)
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.csv'))
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()
  await page.getByRole('button', { name: '确认合并' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.tables?.length).toBe(2)
  await expect(page.getByRole('button', { name: /^收起可变草稿/ })).toContainText('修订 1')
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/multi.xlsx'))
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()
  await page.getByRole('button', { name: '确认合并' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.tables?.some((table: any) => table.tableName === 'Loads')).toBe(true)
  await expect(page.getByRole('button', { name: /^收起可变草稿/ })).toContainText('修订 2')
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/versions`)).json()).versions.length).toBe(2)
  const versions = (await (await api.get(`/api/sessions/${sessionId}/versions`)).json()).versions
  expect(versions[0].tables.map((table: any) => table.tableName)).toEqual(expect.arrayContaining(['bus', 'load', 'Loads']))
  expect(versions.some((version: any) => version.id === firstVersion)).toBe(true)
  const secondVersion = versions[0].id
  const secondTableCount = versions[0].tables.length
  const stale = await api.post(`/api/scripts/${script.id}/runs`, { data: { modelVersionId: firstVersion, clientRunId: 'stale-active' } })
  expect(stale.status()).toBe(409)
  expect((await stale.json()).code).toBe('active_model_changed')

  await page.getByText(`版本 ${firstVersion} · 1 表`, { exact: true }).locator('..').getByRole('button', { name: '激活' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.id).toBe(firstVersion)
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.parquet'))
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()
  await page.getByRole('button', { name: '确认合并' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.revision).toBe(1)
  page.once('dialog', dialog => dialog.accept())
  await page.getByText(`版本 ${secondVersion} · ${secondTableCount} 表`, { exact: true }).locator('..').getByRole('button', { name: '激活' }).click()
  await expect.poll(async () => {
    const state = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
    return state.active?.id === secondVersion && state.draft === null
  }).toBe(true)

  const preview = await api.post(`/api/versions/${firstVersion}/deletion-preview`)
  expect(preview.status()).toBe(200)
  const impact = await preview.json()
  expect(impact.runCount).toBe(1)
  expect((await api.delete(`/api/versions/${firstVersion}?impactToken=${encodeURIComponent(impact.impactToken)}`)).status()).toBe(204)
  expect((await api.get(`/api/versions/${firstVersion}`)).status()).toBe(404)
  expect((await api.get(`/api/runs/${firstRun.id}`)).status()).toBe(404)
  expect((await api.get(`/api/datasets/${firstDataset.id}`)).status()).toBe(404)
})

test('active draft and historical model tables are readable without changing lifecycle state', async ({ appPage }) => {
  const { page, api } = await appPage({ subject: 'rss-replace@example.com' })
  const { sessionId, versionId: firstVersion } = await createReadySession(page, api, '模型表只读查看')
  if (await page.getByRole('button', { name: '展开结果面板' }).isVisible()) {
    await page.getByRole('button', { name: '展开结果面板' }).click()
  }

  await page.getByRole('button', { name: /^展开当前模型/ }).click()
  const activeSection = page.getByRole('button', { name: /^收起当前模型/ }).locator('xpath=../..')
  await activeSection.getByRole('button', { name: '查看 bus' }).click()
  await expect(page.getByRole('dialog', { name: `当前模型 · 版本 ${firstVersion} · bus` })).toBeVisible()
  await expect(page.getByRole('cell', { name: 'b1' })).toBeVisible()
  await page.getByRole('button', { name: '关闭模型表' }).click()

  const csv = ['id,kv,zone', ...Array.from({ length: 101 }, (_, index) => `b${index + 1},${10 + index},zone-${index + 1}`)].join('\n') + '\n'
  await page.locator('input[type=file]').first().setInputFiles({
    name: 'bus.csv', mimeType: 'text/csv', buffer: Buffer.from(csv),
  })
  await expect(page.getByRole('button', { name: '确认合并' })).toBeVisible()
  await page.getByRole('button', { name: '确认合并' }).click()
  let draft: any
  await expect.poll(async () => {
    draft = (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft
    return draft?.revision
  }).toBe(1)

  const firstVersionView = await (await api.get(`/api/versions/${firstVersion}`)).json()
  const activeDataframeId = firstVersionView.tables[0].id
  const activeRows = await (await api.get(`/api/model-dataframes/${activeDataframeId}/rows`)).json()
  expect(activeRows.resourceId).toBe(`model-dataframe:${activeDataframeId}`)
  expect(activeRows.rows).toEqual([['b1', '10.0', 'north']])
  expect((await api.get(`/api/model-dataframes/${activeDataframeId}/download`)).status()).toBe(200)
  const draftDataframeId = draft.tables.find((table: any) => table.tableName === 'bus').id
  const draftRows = await (await api.get(
    `/api/draft-dataframes/${draftDataframeId}/rows?draftId=${draft.id}&expectedDraftRevision=${draft.revision}`,
  )).json()
  expect(draftRows.resourceId).toBe(`draft-dataframe:${draftDataframeId}:revision:${draft.revision}`)
  const draftSection = page.getByRole('button', { name: /^收起可变草稿/ }).locator('xpath=../..')
  await draftSection.getByRole('button', { name: '查看 bus' }).click()
  await expect(page.getByRole('dialog', { name: '可变草稿 · 修订 1 · bus' })).toBeVisible()
  await expect(page.getByRole('cell', { name: 'b100' })).toBeVisible()
  await page.getByRole('button', { name: '选择列' }).click()
  await page.getByRole('checkbox', { name: '显示列 zone' }).uncheck()
  await expect(page.getByRole('columnheader', { name: 'zone' })).toHaveCount(0)
  await page.getByRole('button', { name: '加载更多' }).click()
  await expect(page.getByRole('cell', { name: 'b101' })).toBeVisible()
  const download = page.waitForEvent('download')
  await page.getByRole('button', { name: '下载 Parquet' }).click()
  expect((await download).suggestedFilename()).toContain('bus.parquet')
  await page.getByRole('button', { name: '关闭模型表' }).click()

  await page.getByRole('button', { name: '生成版本' }).click()
  let secondVersion = ''
  await expect.poll(async () => {
    const workspace = await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()
    secondVersion = workspace.active?.id
    return workspace.draft === null && secondVersion !== firstVersion
  }).toBe(true)
  await page.getByRole('button', { name: `版本 ${firstVersion} · 1 表` }).click()
  const historyEntry = page.getByRole('button', { name: `版本 ${firstVersion} · 1 表` }).locator('xpath=../..')
  await historyEntry.getByRole('button', { name: '查看 bus' }).click()
  await expect(page.getByRole('dialog', { name: `历史版本 · 版本 ${firstVersion} · bus` })).toBeVisible()
  await expect(page.getByRole('cell', { name: 'b1' })).toBeVisible()
  expect((await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active.id).toBe(secondVersion)
  expect((await (await api.get(`/api/sessions/${sessionId}/runs`)).json()).runs).toEqual([])
})
