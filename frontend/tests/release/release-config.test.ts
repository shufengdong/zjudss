import { describe, expect, test } from 'vitest'
import releaseConfig from '../../playwright.release.config'

describe('release Playwright configuration', () => {
  test('starts the production backend binary explicitly', () => {
    const servers = Array.isArray(releaseConfig.webServer)
      ? releaseConfig.webServer
      : [releaseConfig.webServer]

    expect(servers[0]?.command).toBe('cargo run --quiet --bin backend')
  })
})
