import { expect, test, type APIRequestContext } from '@playwright/test'
import { createHash, randomUUID } from 'node:crypto'
import { readFile, readdir } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import {
  checkpointCase,
  initializeReport,
  preserveResumeSource,
  sanitizeCaseFailure,
  selectExampleIds,
  validateResumeReport,
  type ManualCaseReport,
  type ManualCaseResult,
} from './report'
import { CASE_PROMPTS, promptForFacade, promptSha256 } from './prompts'
import {
  cleanupSession,
  discoverExamples,
  JourneyPhaseError,
  runCaseJourney,
  type DiscoveredExample,
} from './journey'

test.setTimeout(12 * 60 * 60_000)

async function agentBundleEvidence() {
  const root = resolve(process.cwd(), '../backend/resources/agent/v4')
  const files: string[] = []
  const visit = async (directory: string, prefix = '') => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name
      if (entry.isDirectory()) await visit(join(directory, entry.name), relative)
      else if (entry.isFile()) files.push(relative)
    }
  }
  await visit(root)
  files.sort()
  const hash = createHash('sha256')
  for (const relative of files) {
    hash.update(relative).update('\0').update(await readFile(join(root, relative))).update('\0')
  }
  const manifest = JSON.parse(await readFile(join(root, 'manifest.json'), 'utf8')) as {
    agentBundleVersion: string
  }
  return { version: manifest.agentBundleVersion, sha256: hash.digest('hex') }
}

function parseCsvRecords(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let quoted = false
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]
    if (quoted) {
      if (char === '"' && text[index + 1] === '"') { field += '"'; index += 1 }
      else if (char === '"') quoted = false
      else field += char
    } else if (char === '"') quoted = true
    else if (char === ',') { row.push(field); field = '' }
    else if (char === '\n') { row.push(field.replace(/\r$/, '')); rows.push(row); row = []; field = '' }
    else field += char
  }
  if (field || row.length) { row.push(field.replace(/\r$/, '')); rows.push(row) }
  return rows.filter(values => values.some(value => value.length > 0))
}

async function login(api: APIRequestContext) {
  const records = parseCsvRecords(await readFile(resolve(process.cwd(), '../backend/resources/init_users.csv'), 'utf8'))
  if (records[0]?.[0] !== 'ID' || !records[1]?.[0]) throw new Error('manual_case_user_seed_invalid')
  const userId = records[1][0]
  const response = await api.post('/api/auth/login', { data: { id: userId, password: userId } })
  if (!response.ok()) throw new Error(`manual_case_login_http_${response.status()}`)
}

async function readResumeReport(path: string | undefined): Promise<ManualCaseReport | undefined> {
  if (!path) return undefined
  return JSON.parse(await readFile(resolve(path), 'utf8')) as ManualCaseReport
}

function selectedMap(examples: DiscoveredExample[], read: (example: DiscoveredExample) => string) {
  return Object.fromEntries(examples.map(example => [example.exampleId, read(example)]))
}

function redactions() {
  return Object.entries(process.env)
    .filter(([name, value]) => value
      && /(?:KEY|TOKEN|SECRET|PASSWORD|DATABASE_URL|DATA_DIR|SPARROWZZ_DIR|TENSOREVAL_BIN)/.test(name))
    .map(([, value]) => value as string)
}

test('manually runs every selected case through import, natural-language publish, study, and result', async ({ page }) => {
  const api = page.context().request
  await login(api)
  await page.goto('/')
  await expect(page.getByRole('button', { name: '新建仿真' })).toBeVisible()

  const discovered = await discoverExamples(api)
  const bundle = await agentBundleEvidence()
  const resumeReport = await readResumeReport(process.env.CASE_JOURNEY_RESUME_REPORT)
  if (resumeReport) validateResumeReport(resumeReport, {
    engineResourceSnapshot: discovered.engineResourceSnapshot,
    agentBundleVersion: bundle.version,
    agentBundleSha256: bundle.sha256,
    discoveredExampleIds: discovered.examples.map(example => example.exampleId),
  })
  const selectedIds = selectExampleIds(discovered.examples, {
    exampleId: process.env.CASE_JOURNEY_EXAMPLE_ID,
    facade: process.env.CASE_JOURNEY_FACADE,
  }, resumeReport)
  const selectedSet = new Set(selectedIds)
  const selectedExamples = discovered.examples.filter(example => selectedSet.has(example.exampleId))
  const promptHashes = Object.fromEntries(Object.entries(CASE_PROMPTS)
    .map(([facade, prompt]) => [facade, promptSha256(prompt)]))
  for (const example of discovered.examples) promptForFacade(example.targetFunction)
  const reportPath = resolve(process.env.MANUAL_CASE_REPORT
    ?? '../target/manual-case-journeys/report.json')
  const now = new Date().toISOString()
  const runId = `manual-case-${Date.now()}-${randomUUID().slice(0, 8)}`
  const report: ManualCaseReport = {
    schemaVersion: 1,
    runId,
    resumedFromRunId: resumeReport?.runId,
    engineResourceSnapshot: discovered.engineResourceSnapshot,
    agentBundleVersion: bundle.version,
    agentBundleSha256: bundle.sha256,
    startedAt: now,
    updatedAt: now,
    discoveredExampleIds: discovered.examples.map(example => example.exampleId),
    selectedExampleIds: selectedIds,
    selectedFacades: selectedMap(selectedExamples, example => example.targetFunction),
    selectedBundlePaths: selectedMap(selectedExamples, example => example.bundlePath),
    selectedProjectionProfiles: selectedMap(selectedExamples, example => example.projectionProfile),
    promptHashes,
    cases: [],
  }
  expect(selectedExamples.length, 'manual_case_selection_empty').toBeGreaterThan(0)
  await preserveResumeSource(process.env.CASE_JOURNEY_RESUME_REPORT, reportPath, runId)
  await initializeReport(reportPath, report)

  const failedIds: string[] = []
  for (const example of selectedExamples) {
    const started = Date.now()
    const casePromptSha256 = promptSha256(promptForFacade(example.targetFunction))
    const casePage = await page.context().newPage()
    try {
      await casePage.goto('/')
      await expect(casePage.getByRole('button', { name: '新建仿真' })).toBeVisible()
      const evidence = await runCaseJourney(casePage, api, discovered.engineResourceSnapshot, example)
      const passed: ManualCaseResult = {
        exampleId: example.exampleId,
        bundlePath: example.bundlePath,
        projectionProfile: example.projectionProfile,
        facade: example.targetFunction,
        status: 'passed',
        phase: 'verify_result',
        durationMs: Date.now() - started,
        phaseDurationsMs: evidence.phaseDurationsMs,
        promptSha256: evidence.promptSha256,
        sessionId: evidence.sessionId,
        runId: evidence.runId,
        datasetIds: evidence.datasetIds,
      }
      await checkpointCase(reportPath, passed)
      const cleanupStarted = Date.now()
      try {
        await cleanupSession(casePage, api, evidence.sessionId)
        passed.phaseDurationsMs = {
          ...passed.phaseDurationsMs,
          cleanup_session: Date.now() - cleanupStarted,
        }
        passed.durationMs = Date.now() - started
        await checkpointCase(reportPath, passed)
      } catch (cause) {
        failedIds.push(example.exampleId)
        await checkpointCase(reportPath, {
          ...passed,
          status: 'failed',
          phase: 'cleanup_session',
          durationMs: Date.now() - started,
          phaseDurationsMs: {
            ...passed.phaseDurationsMs,
            cleanup_session: Date.now() - cleanupStarted,
          },
          error: sanitizeCaseFailure(cause, redactions()),
        })
      }
    } catch (cause) {
      failedIds.push(example.exampleId)
      const failure = cause instanceof JourneyPhaseError ? cause : undefined
      await checkpointCase(reportPath, {
        exampleId: example.exampleId,
        bundlePath: example.bundlePath,
        projectionProfile: example.projectionProfile,
        facade: example.targetFunction,
        status: 'failed',
        phase: failure?.phase ?? 'verify_result',
        durationMs: Date.now() - started,
        phaseDurationsMs: failure?.phaseDurationsMs,
        promptSha256: casePromptSha256,
        sessionId: failure?.sessionId,
        error: sanitizeCaseFailure(cause, redactions()),
      })
    } finally {
      await casePage.close().catch(() => {})
    }
  }

  expect(failedIds, `manual_case_failed_ids:${failedIds.join(',')}`).toEqual([])
})
