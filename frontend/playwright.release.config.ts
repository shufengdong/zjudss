import { defineConfig, devices } from '@playwright/test'

const proxyToken = process.env.RELEASE_PROXY_TOKEN ?? 'release-proxy-token'

export default defineConfig({
  testDir: '../integration-tests/release',
  testMatch: 'real-dependencies.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 15 * 60_000,
  expect: { timeout: 30_000 },
  outputDir: '../target/release-integration/playwright-artifacts',
  reporter: [['line']],
  use: {
    baseURL: 'http://127.0.0.1:15183',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    ...devices['Desktop Chrome'],
  },
  webServer: [
    {
      command: 'cargo run --quiet',
      cwd: '../backend',
      url: 'http://127.0.0.1:8080/health',
      reuseExistingServer: false,
      timeout: 180_000,
    },
    {
      command: 'npm run dev -- --host 127.0.0.1 --port 15183 --strictPort',
      cwd: '.',
      env: {
        ...process.env,
        VITE_API_TARGET: 'http://127.0.0.1:8080',
        VITE_E2E_PROXY_TOKEN: proxyToken,
      },
      url: 'http://127.0.0.1:15183',
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
})
