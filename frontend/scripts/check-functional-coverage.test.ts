import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { loadCatalog, validateCatalog } from './check-functional-coverage.mjs'

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

  it('requires lanes, tests and business assertions', async () => {
    const report = await validateCatalog(
      [{
        id: 'BROKEN-001',
        source: 'docs/integration-testing-design.md#1-目标和覆盖口径',
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
})
