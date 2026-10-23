import { describe, expect, test } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import browserConfig from '../../playwright.config'
import releaseConfig from '../../playwright.release.config'

describe('release Playwright configuration', () => {
  test('starts the production backend binary explicitly', () => {
    const servers = Array.isArray(releaseConfig.webServer)
      ? releaseConfig.webServer
      : [releaseConfig.webServer]

    expect(servers[0]?.command).toBe('cargo run --quiet --bin backend')
  })

  test('records test IDs and immutable agent and engine identities', () => {
    const repoRoot = resolve(import.meta.dirname, '../../..')
    const journey = readFileSync(resolve(repoRoot, 'integration-tests/release/real-dependencies.spec.ts'), 'utf8')
    const gate = readFileSync(resolve(repoRoot, 'scripts/test-release-integration'), 'utf8')

    for (const token of ['testIds', 'agentBundleVersion', 'agentBundleSha256', 'engineBinarySha256']) {
      expect(journey).toContain(token)
      expect(gate).toContain(token)
    }
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
