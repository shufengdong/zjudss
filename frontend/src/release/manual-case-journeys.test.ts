import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { afterEach, describe, expect, test } from 'vitest'
import {
  CASE_PROMPTS,
  SUPPORTED_CASE_FACADES,
  promptForFacade,
  promptSha256,
} from '../../../integration-tests/manual-case-journeys/prompts'
import {
  checkpointCase,
  initializeReport,
  sanitizeCaseFailure,
  selectExampleIds,
  type ManualCaseReport,
} from '../../../integration-tests/manual-case-journeys/report'

const temporaryDirectories: string[] = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(directory => rm(directory, { recursive: true, force: true })))
})

function extractGuidePrompt(guide: string, facade: string) {
  const escaped = facade.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const fence = '```'
  const match = guide.match(new RegExp(
    `<!-- CASE_JOURNEY_PROMPT:${escaped}:start -->\\s*${fence}text\\s*([\\s\\S]*?)\\s*${fence}\\s*<!-- CASE_JOURNEY_PROMPT:${escaped}:end -->`,
  ))
  if (!match) throw new Error(`guide_prompt_missing:${facade}`)
  return match[1].trim()
}

function report(): ManualCaseReport {
  return {
    schemaVersion: 1,
    runId: 'manual-1',
    engineResourceSnapshot: 'ers_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    agentBundleVersion: 'v4.1.0',
    agentBundleSha256: 'b'.repeat(64),
    startedAt: '2026-10-30T00:00:00.000Z',
    updatedAt: '2026-10-30T00:00:00.000Z',
    discoveredExampleIds: ['ex_a', 'ex_b', 'ex_c'],
    selectedExampleIds: ['ex_a', 'ex_b', 'ex_c'],
    promptHashes: Object.fromEntries(SUPPORTED_CASE_FACADES.map(facade => [facade, promptSha256(CASE_PROMPTS[facade])])),
    cases: [],
  }
}

describe('manual case journey prompts', () => {
  test('keeps exactly the six published-case facade prompts equal to the tester guide', async () => {
    expect(SUPPORTED_CASE_FACADES).toEqual([
      'ds_newton_raphson_pf',
      'ds_fault_study',
      'ds_network_dynamics',
      'ds_radial_execution_plan',
      'ds_newton_raphson_execution_plan',
      'ds_harmonic_execution_plan',
    ])
    expect(Object.keys(CASE_PROMPTS).sort()).toEqual([...SUPPORTED_CASE_FACADES].sort())
    const guide = await readFile(resolve(import.meta.dirname, '../../../docs/v4-tester-guide.md'), 'utf8')
    for (const facade of SUPPORTED_CASE_FACADES) {
      expect(CASE_PROMPTS[facade]).toBe(extractGuidePrompt(guide, facade))
      expect(promptSha256(CASE_PROMPTS[facade])).toMatch(/^[a-f0-9]{64}$/)
    }
  })

  test('rejects a case target function without a tester-guide prompt', () => {
    expect(() => promptForFacade('ds_radial_pf')).toThrow('unsupported_case_facade:ds_radial_pf')
  })
})

describe('manual case journey report primitives', () => {
  const examples = [
    { exampleId: 'ex_a', importable: true, targetFunction: 'ds_newton_raphson_pf' },
    { exampleId: 'ex_b', importable: true, targetFunction: 'ds_fault_study' },
    { exampleId: 'ex_c', importable: true, targetFunction: 'ds_fault_study' },
    { exampleId: 'ex_invalid', importable: false, targetFunction: 'ds_fault_study' },
  ]

  test('filters by stable example or facade and resumes only failed or unrun cases', () => {
    expect(selectExampleIds(examples, { exampleId: 'ex_b' })).toEqual(['ex_b'])
    expect(selectExampleIds(examples, { facade: 'ds_fault_study' })).toEqual(['ex_b', 'ex_c'])
    expect(selectExampleIds(examples, {}, {
      ...report(),
      cases: [
        { exampleId: 'ex_a', facade: 'ds_newton_raphson_pf', status: 'passed', phase: 'verify_result', durationMs: 10, promptSha256: 'a'.repeat(64) },
        { exampleId: 'ex_b', facade: 'ds_fault_study', status: 'failed', phase: 'wait_workflow', durationMs: 20, promptSha256: 'b'.repeat(64), error: { code: 'engine_timeout', summary: '引擎超时' } },
      ],
    })).toEqual(['ex_b', 'ex_c'])
  })

  test('atomically checkpoints parseable results and derives current counts', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'zjudss-manual-report-'))
    temporaryDirectories.push(directory)
    const reportPath = resolve(directory, 'report.json')
    await initializeReport(reportPath, report())
    await checkpointCase(reportPath, {
      exampleId: 'ex_a', facade: 'ds_newton_raphson_pf', status: 'passed', phase: 'verify_result', durationMs: 10, promptSha256: 'a'.repeat(64),
    })
    let current = JSON.parse(await readFile(reportPath, 'utf8'))
    expect(current.summary).toEqual({ discovered: 3, selected: 3, passed: 1, failed: 0, pending: 2 })
    await checkpointCase(reportPath, {
      exampleId: 'ex_b', facade: 'ds_fault_study', status: 'failed', phase: 'wait_workflow', durationMs: 20, promptSha256: 'b'.repeat(64), error: { code: 'engine_timeout', summary: '引擎超时' },
    })
    current = JSON.parse(await readFile(reportPath, 'utf8'))
    expect(current.cases.map((entry: { exampleId: string }) => entry.exampleId)).toEqual(['ex_a', 'ex_b'])
    expect(current.summary).toEqual({ discovered: 3, selected: 3, passed: 1, failed: 1, pending: 1 })
  })

  test('sanitizes failures without retaining secret path cookie stderr or script fields', () => {
    const failure = sanitizeCaseFailure({
      code: 'engine_timeout',
      message: 'Bearer secret-token failed at /Users/alice/private/model.rss with Cookie: session=abc',
      stderr: 'raw private engine output',
      script: 'result = forbidden();',
    }, ['secret-token', 'session=abc'])
    expect(failure.code).toBe('engine_timeout')
    expect(failure.summary).toContain('<redacted>')
    expect(JSON.stringify(failure)).not.toMatch(/secret-token|session=abc|\/Users\/alice|raw private|forbidden/)
  })
})
