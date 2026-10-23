import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { contractKeys, loadCatalog, validateCatalog } from './check-functional-coverage.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(here, '../..')
const fixture = (name: string) => path.join(repoRoot, 'integration-tests/catalog/fixtures', name)

describe('functional coverage catalog', () => {
  it('accepts a valid catalog without claiming contract coverage', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))

    const report = await validateCatalog(entries, repoRoot)

    expect(report).toMatchObject({
      catalogEntries: 1,
      validEntries: 1,
      missing: [],
      duplicateIds: [],
      contractTotal: null,
      contractCovered: null,
      coveragePercent: null,
    })
  })

  it('rejects duplicate feature ids', async () => {
    const entries = await loadCatalog(fixture('duplicate-id.yaml'))

    const report = await validateCatalog(entries, repoRoot)

    expect(report.duplicateIds).toEqual(['BOOT-001'])
    expect(report.validEntries).toBe(0)
  })

  it('rejects a missing test symbol', async () => {
    const entries = await loadCatalog(fixture('missing-test.yaml'))

    const report = await validateCatalog(entries, repoRoot)

    expect(report.missing).toEqual([
      'BOOT-001: test symbol missing backend/tests/app_bootstrap.rs::does_not_exist',
    ])
    expect(report.validEntries).toBe(0)
  })

  it('rejects text that is present but is not a declared test', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].tests[0].name = 'safe'

    const report = await validateCatalog(entries, repoRoot)

    expect(report.missing).toEqual([
      'BOOT-001: test symbol missing backend/tests/app_bootstrap.rs::safe',
    ])
  })

  it('rejects enumeration-only evidence for a production contract', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].contracts = ['route:GET /health']
    entries[0].tests = [{
      runner: 'rust',
      file: 'backend/tests/contract_manifest.rs',
      name: 'production_router_and_manifest_share_one_declaration',
    }]

    const report = await validateCatalog(entries, repoRoot)

    expect(report.missing).toEqual([
      'BOOT-001: enumeration-only test cannot prove contract behavior backend/tests/contract_manifest.rs::production_router_and_manifest_share_one_declaration',
    ])
  })

  it('rejects an unrelated passing test as route behavior evidence', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].contracts = ['route:GET /health']
    entries[0].tests = [{
      runner: 'playwright',
      file: 'integration-tests/e2e/session-recovery.spec.ts',
      name: 'session CRUD renders optimistic user, tool-call, and assistant messages',
    }]
    const report = await validateCatalog(entries, repoRoot, null, {
      requireRouteEvidence: true,
      routeEvidence: [{
        file: entries[0].tests[0].file,
        name: entries[0].tests[0].name,
        passed: true,
        requests: [{ method: 'GET', path: '/api/sessions', status: 200 }],
      }],
    })

    expect(report.missing).toEqual([
      'BOOT-001: route contract lacks successful runtime evidence route:GET /health',
    ])
  })

  it('accepts a successful matching route response from the declared test', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].contracts = ['route:GET /api/sessions/{id}']
    entries[0].tests = [{
      runner: 'playwright',
      file: 'integration-tests/e2e/session-recovery.spec.ts',
      name: 'session CRUD renders optimistic user, tool-call, and assistant messages',
    }]
    const report = await validateCatalog(entries, repoRoot, null, {
      requireRouteEvidence: true,
      routeEvidence: [{
        file: entries[0].tests[0].file,
        name: entries[0].tests[0].name,
        passed: true,
        requests: [{ method: 'GET', path: '/api/sessions/42', status: 200 }],
      }],
    })

    expect(report.missing).toEqual([])
  })

  it('requires lanes, tests and business assertions', async () => {
    const report = await validateCatalog(
      [{
        id: 'BROKEN-001',
        source: 'docs/integration-testing-design.md#coverage-matrix',
        title: 'broken entry',
        layers: ['api'],
        lanes: [],
        tests: [],
        assertions: [],
        fixtures: [],
      }],
      repoRoot,
    )

    expect(report.missing).toEqual([
      'BROKEN-001: lanes must not be empty',
      'BROKEN-001: tests must not be empty',
      'BROKEN-001: assertions must not be empty',
    ])
    expect(report.validEntries).toBe(0)
  })

  it('reports contract coverage from production manifest keys', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].contracts = ['route:GET /health', 'message:user', 'view:line']

    const report = await validateCatalog(entries, repoRoot, {
      routes: [{ method: 'GET', path: '/health' }],
      messageTypes: ['user'],
      toolNames: ['inspect_workspace'],
      skillIds: ['model-intake'],
      uploadSources: ['single_rss'],
      visualizationKinds: ['line'],
      stableErrorCodes: ['agent_queue_full'],
    })

    expect(report).toMatchObject({
      contractTotal: 7,
      contractCovered: 3,
      coveragePercent: 3 / 7 * 100,
      missingContracts: [
        'error:agent_queue_full',
        'skill:model-intake',
        'tool:inspect_workspace',
        'upload:single_rss',
      ],
    })
  })

  it.each([
    ['route', 'routes'],
    ['tool', 'toolNames'],
    ['message', 'messageTypes'],
    ['view', 'visualizationKinds'],
    ['upload', 'uploadSources'],
    ['skill', 'skillIds'],
    ['error', 'stableErrorCodes'],
  ])('strict PR mode reports an exact missing %s contract', async (_kind, field) => {
    const manifest = {
      routes: [{ method: 'GET', path: '/health' }],
      messageTypes: ['user'],
      toolNames: ['inspect_workspace'],
      skillIds: ['model-intake'],
      uploadSources: ['csv'],
      visualizationKinds: ['line'],
      stableErrorCodes: ['agent_queue_full'],
    }
    const contracts = contractKeys(manifest)
    const removed = contracts.find((key) => key.startsWith(`${_kind}:`))!
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries[0].contracts = contracts.filter((key) => key !== removed)

    const report = await validateCatalog(entries, repoRoot, manifest, { requiredLane: 'pr' })

    expect(report.missingContracts).toEqual([removed])
    expect(report.gatePassed).toBe(false)
    expect(report.coveragePercent).toBeLessThan(100)
    expect(field).toBeTruthy()
  })

  it('strict mode rejects manual-only and a skip without a reason', async () => {
    const manual = await validateCatalog(
      await loadCatalog(fixture('manual-only.yaml')),
      repoRoot,
      null,
      { requiredLane: 'pr' },
    )
    const skipped = await validateCatalog(
      await loadCatalog(fixture('skip-without-reason.yaml')),
      repoRoot,
      null,
      { requiredLane: 'pr' },
    )

    expect(manual.missing).toContain('MANUAL-001: manual-only entries are forbidden')
    expect(skipped.missing).toContain('SKIP-001: skip requires a non-empty reason')
    expect(manual.gatePassed).toBe(false)
    expect(skipped.gatePassed).toBe(false)
  })

  it('release entries require a release-runner test', async () => {
    const report = await validateCatalog(
      await loadCatalog(fixture('release-pr-only.yaml')),
      repoRoot,
      null,
      { requiredLane: 'release' },
    )

    expect(report.missing).toContain('RELEASE-001: release lane requires a release runner test')
    expect(report.gatePassed).toBe(false)
  })

  it('strict mode rejects duplicate and unknown contract declarations', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))
    entries.push({ ...entries[0], id: 'BOOT-002' })
    entries[0].contracts = ['route:GET /health', 'route:GET /unknown']
    entries[1].contracts = ['route:GET /health']

    const report = await validateCatalog(entries, repoRoot, {
      routes: [{ method: 'GET', path: '/health' }],
    }, { requiredLane: 'pr' })

    expect(report.duplicateContracts).toEqual(['route:GET /health'])
    expect(report.unknownContracts).toEqual(['route:GET /unknown'])
    expect(report.gatePassed).toBe(false)
  })

  it('strict mode freezes every top-level functional scope', async () => {
    const entries = await loadCatalog(fixture('valid.yaml'))

    const report = await validateCatalog(entries, repoRoot, null, { requiredLane: 'pr' })

    expect(report.missingFeatureScopes).toContain('MODEL-IMPORT')
    expect(report.gatePassed).toBe(false)
  })
})
