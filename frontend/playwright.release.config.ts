import { defineConfig, devices } from '@playwright/test'

const backendPort = Number(process.env.RELEASE_BACKEND_PORT ?? '18081')
const frontendPort = Number(process.env.RELEASE_FRONTEND_PORT ?? '15183')
const backendOrigin = `http://127.0.0.1:${backendPort}`
const frontendOrigin = `http://127.0.0.1:${frontendPort}`

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
    baseURL: frontendOrigin,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    ...devices['Desktop Chrome'],
  },
  webServer: [
    {
      command: 'cargo run --quiet --bin zjudss',
      cwd: '../backend',
      env: {
        ...process.env,
        BIND_ADDR: `127.0.0.1:${backendPort}`,
      },
      url: `${backendOrigin}/health`,
      reuseExistingServer: false,
      timeout: 180_000,
    },
    {
      command: `npm run dev -- --host 127.0.0.1 --port ${frontendPort} --strictPort`,
      cwd: '.',
      env: {
        ...process.env,
        VITE_API_TARGET: backendOrigin,
      },
      url: frontendOrigin,
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
})
