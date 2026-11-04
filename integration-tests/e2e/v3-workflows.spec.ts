import { resolve } from 'node:path'
import AxeBuilder from '@axe-core/playwright'
import { test, expect } from './fixtures'
import { createReadySession, send, uploadScriptAndRun } from './helpers/journeys'
import { confirmLatestUpload, openMutableDraft } from './helpers/uploads'

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
  const runCard = runHeader.locator('xpath=ancestor::article[1]')
  const detailToggle = runCard.getByRole('button', { name: '展开运行详情' })
  await expect(detailToggle).toHaveAttribute('aria-expanded', 'false')
  await detailToggle.click()
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

test('user-configured charts survive source deletion', async ({ loginAs, control }) => {
  const alice = await loginAs('alice@example.com')
  const bob = await loginAs('bob@example.com')
  await alice.page.setViewportSize({ width: 1280, height: 900 })
  const { sessionId } = await createReadySession(alice.page, alice.api, '用户配置图表')
  const { run } = await uploadScriptAndRun({ page: alice.page, api: alice.api, control, sessionId,
    engine: { kind: 'chart_workbench_success' } })
  const datasets = (await (await alice.api.get(`/api/runs/${run.id}/datasets`)).json()).datasets
  expect(datasets.map((dataset: any) => dataset.name)).toEqual(['a_summary', 'bus_voltage'])
  const source = datasets[1]

  await alice.page.getByRole('button', { name: '运行记录' }).click()
  const runCard = alice.page.getByRole('button', { name: /replacement\.rss.*成功/ }).locator('xpath=ancestor::article[1]')
  await runCard.getByRole('button', { name: '展开运行详情' }).click()
  await runCard.getByRole('button', { name: '查看 bus_voltage' }).click()
  await expect(alice.page.getByRole('columnheader', { name: /^bus/ })).toBeVisible()
  await alice.page.getByRole('button', { name: '新建图表' }).click()

  const measures = alice.page.getByLabel('数值列（可多选）')
  const threePhaseOptions = measures.locator('option').filter({ hasText: /^v_mag\[/ })
  await expect(threePhaseOptions).toHaveCount(3)
  const threePhaseValues = await threePhaseOptions.evaluateAll(options => options.map(option => (option as HTMLOptionElement).value))
  await measures.selectOption(threePhaseValues)
  await alice.page.getByLabel('v_mag[0] 展示名').fill('A相')
  await alice.page.getByLabel('v_mag[1] 展示名').fill('B相')
  await alice.page.getByLabel('v_mag[2] 展示名').fill('C相')
  await alice.page.getByLabel('图表名称').fill('三相电压')
  await alice.page.getByText('显示设置', { exact: true }).click()
  await alice.page.getByLabel('调色板').selectOption('colorblind')
  await alice.page.getByLabel('图例位置').selectOption('right')
  await alice.page.getByLabel('图例方向').selectOption('vertical')
  await alice.page.getByLabel('线型').selectOption('dashed')
  await alice.page.getByLabel('显示数据点').check()
  await alice.page.getByRole('button', { name: '生成预览' }).click()
  await expect(alice.page.getByLabel(/line 图表.*3 个系列.*A相.*B相.*C相.*图例右侧/)).toBeVisible()
  await alice.page.getByRole('button', { name: '保存图表' }).click()
  await expect(alice.page.getByRole('heading', { name: '三相电压' })).toBeVisible()

  await alice.page.reload()
  await alice.page.getByRole('button', { name: '图表', exact: true }).click()
  await alice.page.getByRole('button', { name: '打开 三相电压', exact: true }).click()
  await expect(alice.page.getByLabel(/line 图表.*3 个系列.*A相.*B相.*C相.*图例右侧/)).toBeVisible()
  await alice.page.getByRole('button', { name: '基于此图表新建' }).click()
  await expect(alice.page.getByLabel('图表名称')).toHaveValue('三相电压 副本')
  await alice.page.getByText('显示设置', { exact: true }).click()
  await alice.page.getByLabel('调色板').selectOption('highContrast')
  await alice.page.getByRole('button', { name: '生成预览' }).click()
  await expect(alice.page.getByRole('button', { name: '保存图表' })).toBeEnabled()
  await alice.page.getByRole('button', { name: '保存图表' }).click()
  await expect(alice.page.getByRole('heading', { name: '三相电压 副本' })).toBeVisible()

  const saved = (await (await alice.api.get(`/api/sessions/${sessionId}/dataset-views`)).json()).views
  expect(saved.map((view: any) => view.name).sort()).toEqual(['三相电压', '三相电压 副本'])
  const original = saved.find((view: any) => view.name === '三相电压')
  const copy = saved.find((view: any) => view.name === '三相电压 副本')
  const metadata = await (await alice.api.get(`/api/dataset-views/${original.id}`)).json()
  expect(metadata.config).toMatchObject({ kind: 'line', measures: [
    { column: 'v_mag', path: [0], label: 'A相' },
    { column: 'v_mag', path: [1], label: 'B相' },
    { column: 'v_mag', path: [2], label: 'C相' },
  ], presentation: { palette: 'colorblind', legend: { position: 'right', orientation: 'vertical' }, line: { style: 'dashed', showSymbol: true } } })
  const copyMetadata = await (await alice.api.get(`/api/dataset-views/${copy.id}`)).json()
  expect(copyMetadata.config.presentation.palette).toBe('highContrast')
  const foreignChecks = [
    bob.api.get(`/api/datasets/${source.id}/chart-capabilities`),
    bob.api.post(`/api/datasets/${source.id}/chart-previews`, { data: { config: metadata.config } }),
    bob.api.post(`/api/datasets/${source.id}/views`, { data: { clientOperationId: '70eb0000-0000-4000-8000-000000000011', name: '越权图表', config: metadata.config } }),
    bob.api.get(`/api/dataset-views/${original.id}`),
    bob.api.get(`/api/dataset-views/${original.id}/data`),
    bob.api.delete(`/api/dataset-views/${original.id}`),
    bob.api.get(`/api/sessions/${sessionId}/dataset-views`),
  ]
  for (const response of await Promise.all(foreignChecks)) expect(response.status()).toBe(404)

  await alice.page.getByRole('button', { name: '关闭图表' }).click()
  await alice.page.getByRole('button', { name: '运行记录' }).click()
  await runCard.getByRole('button', { name: '展开运行详情' }).click()
  await runCard.getByRole('button', { name: '查看 bus_voltage' }).click()
  alice.page.once('dialog', dialog => void dialog.accept())
  await alice.page.getByRole('button', { name: '删除结果' }).click()
  await expect.poll(async () => (await alice.api.get(`/api/datasets/${source.id}`)).status()).toBe(404)

  await alice.page.setViewportSize({ width: 390, height: 844 })
  await alice.page.reload()
  await alice.page.getByRole('button', { name: '展开结果面板' }).click()
  await alice.page.getByRole('button', { name: '图表', exact: true }).click()
  await expect(alice.page.getByText('来源已删除').first()).toBeVisible()
  await alice.page.getByRole('button', { name: '打开 三相电压', exact: true }).click()
  await expect(alice.page.getByLabel(/line 图表.*3 个系列.*A相.*B相.*C相.*图例底部/)).toBeVisible()
  expect(await alice.page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  const axe = await new AxeBuilder({ page: alice.page }).analyze()
  expect(axe.violations.filter(violation => violation.impact === 'serious' || violation.impact === 'critical')).toEqual([])

  const beforeDelete = await (await alice.api.get('/api/storage-usage')).json()
  alice.page.once('dialog', dialog => void dialog.accept())
  await alice.page.getByRole('button', { name: '删除图表' }).click()
  await expect(alice.page.getByRole('button', { name: '打开 三相电压', exact: true })).toHaveCount(0)
  const afterFirstDelete = await (await alice.api.get('/api/storage-usage')).json()
  expect(afterFirstDelete.artifactCount).toBe(beforeDelete.artifactCount)
  expect(afterFirstDelete.logicalBytes).toBe(beforeDelete.logicalBytes)
  alice.page.once('dialog', dialog => void dialog.accept())
  await alice.page.getByRole('button', { name: '删除 三相电压 副本' }).click()
  await expect(alice.page.getByText('暂无已保存图表')).toBeVisible()
  const afterDelete = await (await alice.api.get('/api/storage-usage')).json()
  expect(afterDelete.artifactCount).toBe(beforeDelete.artifactCount - 1)
  expect(afterDelete.logicalBytes).toBeLessThan(beforeDelete.logicalBytes)
})

test('failed run progress shows persistent sanitized tensoreval output without requesting datasets', async ({ appPage, control }) => {
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
  let progressCard = page.getByText('运行进度', { exact: true }).locator('xpath=ancestor::div[contains(@class,"rounded-xl")][1]')
  await expect(progressCard.getByRole('heading', { name: '失败诊断' })).toBeVisible()
  const progressLog = progressCard.locator('pre')
  await expect(progressLog).toHaveText('RSS parse error: unexpected token\nat [服务器路径已隐藏]')
  await expect(progressCard).not.toContainText('/srv/private')

  await page.reload()
  progressCard = page.getByText('运行进度', { exact: true }).locator('xpath=ancestor::div[contains(@class,"rounded-xl")][1]')
  await expect(progressCard.getByRole('heading', { name: '失败诊断' })).toBeVisible()
  await expect(progressCard.locator('pre')).toHaveText('RSS parse error: unexpected token\nat [服务器路径已隐藏]')
  await expect(progressCard).not.toContainText('/srv/private')

  await page.getByRole('button', { name: '运行记录' }).click()
  const runHeader = page.getByRole('button', { name: /replacement\.rss.*失败/ })
  const failedRunCard = runHeader.locator('xpath=ancestor::article[1]')
  const detail = failedRunCard.getByRole('alert')
  await failedRunCard.getByRole('button', { name: '展开运行详情' }).click()
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

test('analysis and comparisons survive source-result deletion with tombstone lineage', async ({ appPage, control }) => {
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
  expect((await api.get(`/api/run-comparisons/${comparisonMessage.comparisonId}`)).status()).toBe(200)
  const lineage = await (await api.get(`/api/datasets/${derivedId}`)).json()
  expect(lineage.lineage[0].deleted).toBe(true)
  expect(lineage.lineage[0].tombstone.datasetId).toBe(source.id)
})

test('CSV imports merge by whole table, reruns use the latest active model, and history activation/deletion is explicit', async ({ appPage, control }) => {
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
  await confirmLatestUpload(page, 'model_table', { 'load.csv': 'load' })
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.tables?.length).toBe(2)
  await expect(await openMutableDraft(page)).toContainText('修订 1')
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/multi.xlsx'))
  await confirmLatestUpload(page, 'model_table', { Mixed: 'Mixed', Loads: 'Loads', DuplicateColumns: 'DuplicateColumns' })
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.tables?.some((table: any) => table.tableName === 'loads')).toBe(true)
  await expect(await openMutableDraft(page)).toContainText('修订 2')
  await page.getByRole('button', { name: '生成版本' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/versions`)).json()).versions.length).toBe(2)
  const versions = (await (await api.get(`/api/sessions/${sessionId}/versions`)).json()).versions
  expect(versions[0].tables.map((table: any) => table.tableName)).toEqual(expect.arrayContaining(['bus', 'load', 'loads']))
  expect(versions.some((version: any) => version.id === firstVersion)).toBe(true)
  const secondVersion = versions[0].id
  const secondTableCount = versions[0].tables.length
  await control.enqueueEngine({ kind: 'success', data: [{ name: 'latest_result', rows: [['x', 2]], valueUnit: null }] })
  const rerun = await api.post(`/api/scripts/${script.id}/runs`, { data: { clientRunId: 'latest-active' } })
  expect(rerun.status()).toBe(202)
  const rerunId = ((await rerun.json()) as { run: { id: string } }).run.id
  await expect.poll(async () => (await (await api.get(`/api/runs/${rerunId}`)).json()).status).toBe('succeeded')
  expect((await (await api.get(`/api/runs/${rerunId}`)).json()).modelVersionId).toBe(secondVersion)

  await page.getByText(`版本 ${firstVersion} · 1 表`, { exact: true }).locator('..').getByRole('button', { name: '激活' }).click()
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).active?.id).toBe(firstVersion)
  await page.locator('input[type=file]').first().setInputFiles(resolve(process.cwd(), '../integration-tests/fixtures/uploads/dataframes/load.parquet'))
  await confirmLatestUpload(page, 'model_table', { 'load.parquet': 'load' })
  await expect.poll(async () => (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft?.revision).toBe(1)
  await openMutableDraft(page)
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
  await confirmLatestUpload(page, 'model_table', { 'bus.csv': 'bus' })
  let draft: any
  await expect.poll(async () => {
    draft = (await (await api.get(`/api/sessions/${sessionId}/workspace`)).json()).draft
    return draft?.revision
  }).toBe(1)
  await openMutableDraft(page)

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
