import { readFile, mkdtemp, rm } from 'node:fs/promises'
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
  preserveResumeSource,
  sanitizeCaseFailure,
  selectExampleIds,
  validateResumeReport,
  type ManualCaseReport,
} from '../../../integration-tests/manual-case-journeys/report'
import { validateManualCaseEnvironment } from '../../manual-case-guard'

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
    agentBundleVersion: 'v4.3.0',
    agentBundleSha256: 'b'.repeat(64),
    startedAt: '2026-10-30T00:00:00.000Z',
    updatedAt: '2026-10-30T00:00:00.000Z',
    discoveredExampleIds: ['ex_a', 'ex_b', 'ex_c'],
    selectedExampleIds: ['ex_a', 'ex_b', 'ex_c'],
    selectedFacades: { ex_a: 'ds_newton_raphson_pf', ex_b: 'ds_fault_study', ex_c: 'ds_fault_study' },
    selectedBundlePaths: { ex_a: 'a', ex_b: 'b', ex_c: 'c' },
    selectedProjectionProfiles: { ex_a: 'pf', ex_b: 'fault', ex_c: 'fault' },
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
        { exampleId: 'ex_a', bundlePath: 'a', projectionProfile: 'pf', facade: 'ds_newton_raphson_pf', status: 'passed', phase: 'verify_result', durationMs: 10, promptSha256: 'a'.repeat(64) },
        { exampleId: 'ex_b', bundlePath: 'b', projectionProfile: 'fault', facade: 'ds_fault_study', status: 'failed', phase: 'wait_workflow', durationMs: 20, promptSha256: 'b'.repeat(64), error: { code: 'engine_timeout', summary: '引擎超时' } },
      ],
    })).toEqual(['ex_b', 'ex_c'])
  })

  test('atomically checkpoints parseable results and derives current counts', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'zjudss-manual-report-'))
    temporaryDirectories.push(directory)
    const reportPath = resolve(directory, 'report.json')
    await initializeReport(reportPath, report())
    await checkpointCase(reportPath, {
      exampleId: 'ex_a', bundlePath: 'a', projectionProfile: 'pf', facade: 'ds_newton_raphson_pf', status: 'passed', phase: 'verify_result', durationMs: 10, promptSha256: 'a'.repeat(64),
    })
    let current = JSON.parse(await readFile(reportPath, 'utf8'))
    expect(current.summary).toEqual({ discovered: 3, selected: 3, passed: 1, failed: 0, pending: 2 })
    expect(current.facadeSummary).toEqual({
      ds_fault_study: { selected: 2, passed: 0, failed: 0, pending: 2 },
      ds_newton_raphson_pf: { selected: 1, passed: 1, failed: 0, pending: 0 },
    })
    await checkpointCase(reportPath, {
      exampleId: 'ex_b', bundlePath: 'b', projectionProfile: 'fault', facade: 'ds_fault_study', status: 'failed', phase: 'wait_workflow', durationMs: 20, promptSha256: 'b'.repeat(64), error: { code: 'engine_timeout', summary: '引擎超时' },
    })
    current = JSON.parse(await readFile(reportPath, 'utf8'))
    expect(current.cases.map((entry: { exampleId: string }) => entry.exampleId)).toEqual(['ex_a', 'ex_b'])
    expect(current.summary).toEqual({ discovered: 3, selected: 3, passed: 1, failed: 1, pending: 1 })
    expect(current.facadeSummary.ds_fault_study).toEqual({ selected: 2, passed: 0, failed: 1, pending: 1 })
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

  test('rejects incompatible resume evidence and preserves a same-path source', async () => {
    const current = report()
    const evidence = {
      engineResourceSnapshot: current.engineResourceSnapshot,
      agentBundleVersion: current.agentBundleVersion,
      agentBundleSha256: current.agentBundleSha256,
      discoveredExampleIds: current.discoveredExampleIds,
    }
    expect(() => validateResumeReport(current, evidence)).not.toThrow()
    expect(() => validateResumeReport(current, {
      ...evidence,
      engineResourceSnapshot: 'ers_different',
    })).toThrow('manual_case_resume_engine_snapshot_mismatch')
    expect(() => validateResumeReport({
      ...current,
      selectedExampleIds: [...current.selectedExampleIds, 'ex_missing'],
    }, evidence)).toThrow('manual_case_resume_selection_invalid')

    const directory = await mkdtemp(resolve(tmpdir(), 'zjudss-manual-resume-'))
    temporaryDirectories.push(directory)
    const reportPath = resolve(directory, 'report.json')
    await initializeReport(reportPath, current)
    const backup = await preserveResumeSource(reportPath, reportPath, 'next-run')
    expect(backup).toBe(`${reportPath}.before-next-run.json`)
    expect(JSON.parse(await readFile(backup!, 'utf8')).runId).toBe(current.runId)
  })

  test('redacts complete headers query secrets broader paths and explicit engine locations', () => {
    const failure = sanitizeCaseFailure({
      message: 'Authorization: Bearer abc\nCookie: a=1; b=2\nGET /x?token=query-secret at /opt/tensoreval/bin and /usr/local/share/case',
      cause: new Error('nested-secret at /private/engine/source.rss'),
    }, ['/engine/private/sparrowzz'])
    expect(failure.summary).not.toMatch(/abc|a=1|b=2|query-secret|nested-secret|\/opt\/|\/usr\//)
    expect(failure.summary).toContain('<redacted>')
  })
})

describe('manual case execution guard', () => {
  test('rejects direct execution and accepts only wrapper-scoped storage', () => {
    expect(() => validateManualCaseEnvironment({})).toThrow('manual_case_explicit_opt_in_required')
    const root = resolve(import.meta.dirname, '../../../target/manual-case-journeys/run.unit')
    const valid = {
      MANUAL_CASE_OPT_IN: '1',
      MANUAL_CASE_RUN_ROOT: root,
      DATABASE_URL: `sqlite://${root}/manual-cases.db`,
      DATA_DIR: `${root}/data`,
    }
    expect(validateManualCaseEnvironment(valid)).toBe(root)
    expect(() => validateManualCaseEnvironment({ ...valid, DATA_DIR: '/tmp/shared-data' }))
      .toThrow('manual_case_data_dir_invalid')
    expect(() => validateManualCaseEnvironment({ ...valid, DATABASE_URL: 'sqlite://existing.db' }))
      .toThrow('manual_case_database_url_invalid')
  })
})
