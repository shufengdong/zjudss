import fs from 'node:fs'
import path from 'node:path'

import YAML from 'yaml'

const repoRoot = path.resolve(import.meta.dirname, '../../..')

function workflow(name: string) {
  return YAML.parse(fs.readFileSync(path.join(repoRoot, '.github/workflows', name), 'utf8'))
}

function commands(job: Record<string, unknown>) {
  return ((job.steps as Array<{ run?: string }> | undefined) ?? [])
    .map((step) => step.run ?? '')
    .join('\n')
}

describe('candidate release workflows', () => {
  it('runs L0 L1 L2 and the strict PR catalog gate', () => {
    const ci = workflow('ci.yml')
    const job = ci.jobs['pr-gate']
    const run = commands(job)

    expect(ci.on).toHaveProperty('pull_request')
    expect(ci.on).toHaveProperty('workflow_call')
    expect(run).toContain('cargo test --features e2e-test')
    expect(run).toContain('npm run test:e2e')
    expect(run).toContain('./scripts/check-functional-coverage --require-pr')
    expect(JSON.stringify(job.steps)).toContain('actions/upload-artifact@v4')
    expect(JSON.stringify(job.steps)).not.toContain('continue-on-error')
  })

  it('requires the protected environment, PR gate, engine artifact and strict release gate', () => {
    const release = workflow('release-integration.yml')
    const job = release.jobs['release-gate']
    const run = commands(job)

    expect(release.jobs['pr-gate'].uses).toBe('./.github/workflows/ci.yml')
    expect(job.needs).toBe('pr-gate')
    expect(job.environment).toBe('candidate-release')
    expect(job.env.DEEPSEEK_API_KEY).toContain('secrets.DEEPSEEK_API_KEY')
    expect(job.env.EXPECTED_TENSOREVAL_SHA256).toContain('vars.TENSOREVAL_SHA256')
    expect(JSON.stringify(job.steps)).toContain('actions/download-artifact@v4')
    expect(run).toContain('sha256sum "$TENSOREVAL_BIN"')
    expect(run).toContain('EXPECTED_TENSOREVAL_SHA256')
    expect(run).toContain('./scripts/test-release-integration')
    expect(run).toContain('./scripts/check-functional-coverage --require-release')
    expect(JSON.stringify(job.steps)).not.toContain('continue-on-error')
  })
})
