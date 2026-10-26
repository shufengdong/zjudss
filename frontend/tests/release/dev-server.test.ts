import { afterAll, beforeAll, describe, expect, test } from 'vitest'
import { createServer, type ViteDevServer } from 'vite'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import type { AddressInfo } from 'node:net'

describe('development server module graph', () => {
  let server: ViteDevServer
  let origin: string
  let cacheDir: string

  beforeAll(async () => {
    const frontendRoot = resolve(import.meta.dirname, '../..')
    cacheDir = await mkdtemp(resolve(tmpdir(), 'zjudss-vite-test-'))
    server = await createServer({
      root: frontendRoot,
      configFile: resolve(frontendRoot, 'vite.config.ts'),
      cacheDir,
      logLevel: 'silent',
      server: { host: '127.0.0.1', port: 0, strictPort: false },
    })
    await server.listen()
    const address = server.httpServer?.address() as AddressInfo
    origin = `http://127.0.0.1:${address.port}`
  })

  afterAll(async () => {
    await server.close()
    await rm(cacheDir, { recursive: true, force: true })
  })

  test('serves the API client runtime dependencies without proxying them to the backend', async () => {
    const clientResponse = await fetch(`${origin}/src/api/client.ts`)
    expect(clientResponse.status).toBe(200)

    const clientModule = await clientResponse.text()
    const configImport = clientModule.match(/from ["']([^"']*api-config[^"']*)["']/)?.[1]
    expect(configImport).toBeDefined()

    const configResponse = await fetch(new URL(configImport!, origin))
    expect(configResponse.status).toBe(200)
    expect(await configResponse.text()).toContain('export function normalizeApiBasePath')
  })
})
