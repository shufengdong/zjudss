import { test, expect } from './fixtures'
import { createSessionFromPage } from './helpers/journeys'
import { newtonScript, prepareNewtonStudy, publishExampleModel, settleNewtonConfiguration } from './helpers/managedStudies'
import { send } from './helpers/journeys'

test('managed Newton study confirms, repairs one engine failure, and publishes only the successful datasets', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'studies@example.com' })
  const sessionId = await createSessionFromPage(page, '受管牛顿潮流')
  await publishExampleModel(page, api, sessionId)
  const configuration = await prepareNewtonStudy(page, api, sessionId, control)
  let configured = await (await api.get(`/api/study-configurations/${configuration.id}`)).json() as any
  let edited = await api.put(`/api/study-configurations/${configuration.id}/facade`, { data: { expectedRevision: configured.revision, facadeId: configured.facade.id } })
  expect(edited.status()).toBe(200)
  configured = await edited.json()
  const para = configured.inputs.find((input: any) => input.inputId === 'para')
  edited = await api.put(`/api/study-configurations/${configuration.id}/bindings/para`, { data: { expectedRevision: configured.revision, resourceId: para.binding.resourceId } })
  expect(edited.status()).toBe(200)
  configured = await edited.json()
  edited = await api.put(`/api/study-configurations/${configuration.id}/structs/para`, { data: { expectedRevision: configured.revision, values: configured.inputs.find((input: any) => input.inputId === 'para').structCard.values } })
  expect(edited.status()).toBe(200)
  configured = await edited.json()
  edited = await api.put(`/api/study-configurations/${configuration.id}/results`, { data: { expectedRevision: configured.revision, results: configured.results.selected } })
  expect(edited.status()).toBe(200)
  configured = await edited.json()
  expect(configured.status, JSON.stringify(configured, null, 2)).toBe('ready_to_confirm')
  await page.reload()
  await expect(configuration.card).toContainText(`修订 ${configured.revision}`)
  await settleNewtonConfiguration(page, configuration.card)

  const baseline = await control.trace() as { author: string[]; authorRepairCount: number; engine: Array<{ scenario: string }> }
  await control.enqueueAuthor({ kind: 'script', content: newtonScript })
  await control.enqueueEngine({ kind: 'run_failure' })
  await control.enqueueAuthor({ kind: 'script', content: newtonScript })
  await control.enqueueEngine({ kind: 'bus_voltage_success' })

  const confirmResponse = page.waitForResponse(response => response.request().method() === 'POST' && /\/api\/study-configurations\/\d+\/confirm$/.test(new URL(response.url()).pathname))
  await configuration.card.getByRole('button', { name: '确认并生成脚本' }).click()
  const confirmed = await confirmResponse
  expect(confirmed.status()).toBe(200)
  const confirmedWorkflow = await confirmed.json() as { id: number }

  await expect.poll(async () => {
    const response = await api.get(`/api/workflows/${confirmedWorkflow.id}`)
    const body = await response.json() as { status: string }
    return body.status
  }, { timeout: 30_000 }).toMatch(/succeeded|failed/)
  const workflowBody = await (await api.get(`/api/workflows/${confirmedWorkflow.id}`)).json() as { status: string }
  expect(workflowBody.status, JSON.stringify({ workflow: workflowBody, trace: await control.trace() }, null, 2)).toBe('succeeded')

  const workflow = page.getByRole('article', { name: '研究工作流进度' }).last()
  await expect(workflow).toContainText('succeeded')
  await expect(workflow).toContainText('结果数据集 #')
  await expect(workflow).not.toContainText('scenario_run_failed')
  const trace = await control.trace() as { author: string[]; authorRepairCount: number; engine: Array<{ scenario: string }> }
  expect(trace.author.slice(baseline.author.length)).toEqual(['generate', 'repair'])
  expect(trace.authorRepairCount - baseline.authorRepairCount).toBe(1)
  expect(trace.engine.slice(baseline.engine.length).map(call => call.scenario)).toEqual(['run_failure', 'bus_voltage_success'])

  const runs = (await (await api.get(`/api/sessions/${sessionId}/runs`)).json() as { runs: Array<{ status: string }> }).runs
  expect(runs).toHaveLength(1)
  expect(runs[0].status).toBe('succeeded')

  const disposable = await prepareNewtonStudy(page, api, sessionId, control)
  const disposableView = await (await api.get(`/api/study-configurations/${disposable.id}`)).json() as { revision: number }
  expect((await api.delete(`/api/study-configurations/${disposable.id}?expectedRevision=${disposableView.revision}`)).status()).toBe(200)
})

test('study configuration maps a generic data resource and can be cancelled', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'dataframe-ready@example.com' })
  const sessionId = await createSessionFromPage(page, '时序输入映射')
  await publishExampleModel(page, api, sessionId)
  const uploaded = await api.post(`/api/sessions/${sessionId}/upload-batches`, { multipart: {
    clientBatchId: `shape-${Date.now()}`,
    'file.shape': { name: 'shape.csv', mimeType: 'text/csv', buffer: Buffer.from('profile,seconds,multiplier\ndaily,0,1.0\ndaily,3600,0.8\n') },
  } })
  expect(uploaded.status()).toBe(202)
  const batch = await uploaded.json() as { uploadBatch: { id: string; candidates: Array<{ id: string }> } }
  const decision = { candidateId: batch.uploadBatch.candidates[0].id, purpose: 'data_resource', selected: true, displayName: '曲线源', alias: 'shape_source', tableName: null }
  const confirmed = await api.post(`/api/upload-batches/${batch.uploadBatch.id}/confirm`, { data: { decisions: [decision], clientOperationId: `shape-confirm-${Date.now()}` } })
  expect(confirmed.status()).toBe(200)
  const sourceId = (await confirmed.json() as { uploadConfirmation: { resources: Array<{ id: string }> } }).uploadConfirmation.resources[0].id

  await control.enqueueLlm({
    id: `qsts-${Date.now()}`,
    expectedPrompt: '准备时序潮流',
    turns: [
      { kind: 'tool_call', call_id: 'inspect-qsts', name: 'inspect_workspace', args: {} },
      { kind: 'tool_call', call_id: 'skill-qsts', name: 'load_agent_skill', args: { skill_id: 'study-planning' } },
      { kind: 'tool_call', call_id: 'search-qsts', name: 'search_study_facades', args: { query: '时序潮流', explicitFunction: 'ds_radial_qsts' } },
      { kind: 'tool_call', call_id: 'prepare-qsts', name: 'prepare_study', args: { facadeId: 'qsts_radial', intent: { request: '准备时序潮流' } } },
      { kind: 'text', text: '时序研究配置已准备。' },
    ],
  })
  await send(page, '准备时序潮流')
  await expect(page.getByText('时序研究配置已准备。')).toBeVisible()
  const messages = await (await api.get(`/api/sessions/${sessionId}/messages`)).json() as { messages: Array<any> }
  const configMessage = [...messages.messages].reverse().find(message => message.type === 'study-config')
  const configurationId = configMessage.card.configurationId as string
  let configuration = await (await api.get(`/api/study-configurations/${configurationId}`)).json() as any
  const mapped = await api.post(`/api/study-configurations/${configurationId}/bindings/load_shape/mapping`, { data: {
    expectedRevision: configuration.revision,
    sourceResourceId: sourceId,
    displayName: '日负荷曲线',
    alias: 'daily_shape',
    mappings: [
      { source: 'profile', target: 'name' },
      { source: 'seconds', target: 'time_s' },
      { source: 'multiplier', target: 'p_multiplier' },
    ],
  } })
  const mappedText = await mapped.text()
  expect(mapped.status(), mappedText).toBe(200)
  configuration = JSON.parse(mappedText)
  expect((await api.delete(`/api/study-configurations/${configurationId}?expectedRevision=${configuration.revision}`)).status()).toBe(200)
})

test('failed managed script previews changed inputs and retries only after confirmation', async ({ appPage, control }) => {
  const { page, api } = await appPage({ subject: 'studies@example.com' })
  const sessionId = await createSessionFromPage(page, '失败研究重试')
  await publishExampleModel(page, api, sessionId)
  const configuration = await prepareNewtonStudy(page, api, sessionId, control)

  for (let attempt = 0; attempt < 4; attempt += 1) {
    await control.enqueueAuthor({ kind: 'script', content: newtonScript })
    await control.enqueueEngine({ kind: 'run_failure' })
  }
  const confirmResponse = page.waitForResponse(response => response.request().method() === 'POST' && /\/api\/study-configurations\/\d+\/confirm$/.test(new URL(response.url()).pathname))
  await configuration.card.getByRole('button', { name: '确认并生成脚本' }).click()
  const source = await (await confirmResponse).json() as { id: number }
  await expect.poll(async () => ((await (await api.get(`/api/workflows/${source.id}`)).json()) as { status: string }).status, { timeout: 30_000 }).toBe('failed')
  const failed = await (await api.get(`/api/workflows/${source.id}`)).json() as { finalScriptResourceId: number }
  const frozenConfiguration = await (await api.get(`/api/study-configurations/${configuration.id}`)).json() as any
  const paraId = frozenConfiguration.inputs.find((input: any) => input.inputId === 'para').binding.resourceId as string
  expect(paraId).toBeTruthy()

  const para = await (await api.get(`/api/data-resources/${paraId}`)).json() as { contentHash: string }
  const replacement = await api.post(`/api/data-resources/${paraId}/structured-replacement-previews`, { data: {
    expectedContentHash: para.contentHash,
    schemaRef: 'frames.para',
    values: { base_frequency_hz: 50, max_iter: 120, max_control_iter: 30, tol: 1e-6, line_model: 'modified' },
  } })
  expect(replacement.status(), await replacement.text()).toBe(200)
  const replacementBody = await replacement.json() as { previewToken: string }
  const replacementConfirmation = await api.post(`/api/data-resource-replacement-previews/${replacementBody.previewToken}/confirm`, { data: { action: 'replace' } })
  const replacementConfirmationText = await replacementConfirmation.text()
  expect(replacementConfirmation.status(), replacementConfirmationText).toBe(200)
  const replacementResource = (JSON.parse(replacementConfirmationText) as { resource: { id: string; contentHash: string } }).resource
  expect(replacementResource.id).not.toBe(paraId)
  expect(replacementResource.contentHash).not.toBe(para.contentHash)

  const preview = await api.post(`/api/scripts/${failed.finalScriptResourceId}/retry-previews`, { data: { clientRetryId: `retry-${Date.now()}` } })
  const previewText = await preview.text()
  expect(preview.status(), previewText).toBe(200)
  const previewBody = JSON.parse(previewText) as { retryPreviewId: string; changes: unknown[]; workflow: null }
  expect(previewBody.retryPreviewId, previewText).toBeTruthy()
  expect(previewBody.changes.length).toBeGreaterThan(0)
  expect(previewBody.workflow).toBeNull()

  await control.enqueueAuthor({ kind: 'script', content: newtonScript })
  await control.enqueueEngine({ kind: 'bus_voltage_success' })
  const confirmed = await api.post(`/api/retry-previews/${previewBody.retryPreviewId}/confirm`, { data: { confirmed: true } })
  const confirmedText = await confirmed.text()
  expect(confirmed.status(), confirmedText).toBe(200)
  const retry = JSON.parse(confirmedText) as { id: number; sourceWorkflowId: number }
  expect(retry.sourceWorkflowId).toBe(source.id)
  await expect.poll(async () => ((await (await api.get(`/api/workflows/${retry.id}`)).json()) as { status: string }).status, { timeout: 30_000 }).toBe('succeeded')
})
