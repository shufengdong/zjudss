import { describe, expect, test } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import browserConfig from '../../playwright.config'
import releaseConfig from '../../playwright.release.config'

describe('release Playwright configuration', () => {
  test('repository release contract is V4-only', () => {
    const root = resolve(import.meta.dirname, '../../..')
    const manifest = JSON.parse(readFileSync(resolve(root, 'backend/resources/agent/v4/manifest.json'), 'utf8'))
    expect(manifest.agentBundleVersion).toBe('v4.0.0')
    expect(manifest.allowedTools).toHaveLength(22)
    expect(manifest.allowedTools).toContain('search_study_facades')
    expect(manifest.allowedTools).toContain('prepare_study')
    expect(manifest.allowedTools).not.toContain('plan_study')
    expect(manifest.allowedTools).not.toContain('create_script')

    const env = readFileSync(resolve(root, 'backend/.env.example'), 'utf8')
    expect(env).toContain('USER_ARTIFACT_LIMIT')
    expect(env).toContain('USER_STORAGE_LIMIT_BYTES')
    expect(env).toContain('AGENT_MAX_USER_RUNS')
    expect(env).not.toMatch(/OWNER_ARTIFACT_LIMIT|OWNER_STORAGE_LIMIT_BYTES|AGENT_MAX_OWNER_RUNS/)
    expect(readFileSync(resolve(root, 'README.md'), 'utf8')).toContain('配电网仿真智能体 V4')
    expect(readFileSync(resolve(root, 'AGENTS.md'), 'utf8')).toContain('docs/v4-deployment.md')
    expect(readFileSync(resolve(root, 'scripts/test-release-engine'), 'utf8')).toContain('--test process_engine_v4')
    expect(readFileSync(resolve(root, 'backend/Cargo.toml'), 'utf8')).toContain('release-test = ["e2e-test"]')
    const releaseJourney = readFileSync(resolve(root, 'integration-tests/release/real-dependencies.spec.ts'), 'utf8')
    expect(releaseJourney).toContain('complete the V4 semantic release journey')
    expect(releaseJourney).toContain("['prepare_study']")
    expect(releaseJourney).not.toMatch(/plan_study|create_script|确认合并|resources\/agent\/v3/)
  })

  test('starts the production backend binary explicitly', () => {
    const servers = Array.isArray(releaseConfig.webServer)
      ? releaseConfig.webServer
      : [releaseConfig.webServer]

    expect(servers[0]?.command).toBe('cargo run --quiet --bin zjudss')
    expect(servers[0]?.url).not.toBe('http://127.0.0.1:8080/health')
    expect(servers[0]?.env?.BIND_ADDR).toBe('127.0.0.1:18081')
  })

  test('records test IDs and immutable agent and engine identities', () => {
    const repoRoot = resolve(import.meta.dirname, '../../..')
    const journey = readFileSync(resolve(repoRoot, 'integration-tests/release/real-dependencies.spec.ts'), 'utf8')
    const gate = readFileSync(resolve(repoRoot, 'scripts/test-release-integration'), 'utf8')

    for (const token of ['testIds', 'agentBundleVersion', 'agentBundleSha256', 'engineBinarySha256']) {
      expect(journey).toContain(token)
      expect(gate).toContain(token)
    }
    expect(journey).toContain('browser.newContext({ baseURL })')
    expect(gate).toContain('RELEASE_BACKEND_PORT')
    expect(gate).toContain('RELEASE_FRONTEND_PORT')
  })
})

describe('PR Playwright configuration', () => {
  test('allows a cold Rust build before the browser journey starts', () => {
    const servers = Array.isArray(browserConfig.webServer)
      ? browserConfig.webServer
      : [browserConfig.webServer]

    expect(servers[0]?.timeout).toBeGreaterThanOrEqual(300_000)
  })
})
