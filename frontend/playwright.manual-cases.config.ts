import { defineConfig, devices } from '@playwright/test'
import { validateManualCaseEnvironment } from './manual-case-guard'

if (!process.argv.includes('--list')) {
  validateManualCaseEnvironment(process.env)
}

const backendPort = Number(process.env.MANUAL_CASE_BACKEND_PORT ?? '18091')
const frontendPort = Number(process.env.MANUAL_CASE_FRONTEND_PORT ?? '15193')
const backendOrigin = `http://127.0.0.1:${backendPort}`
const frontendOrigin = `http://127.0.0.1:${frontendPort}`
const runRoot = process.env.MANUAL_CASE_RUN_ROOT

export default defineConfig({
  testDir: '../integration-tests/manual-case-journeys',
  testMatch: 'all-cases.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 12 * 60 * 60_000,
  expect: { timeout: 120_000 },
  globalSetup: './manual-case-global-setup.ts',
  outputDir: runRoot
    ? `${runRoot}/playwright-artifacts`
    : '../target/manual-case-journeys/unstarted-playwright-artifacts',
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
      env: { ...process.env, BIND_ADDR: `127.0.0.1:${backendPort}` },
      url: `${backendOrigin}/health`,
      reuseExistingServer: false,
      timeout: 300_000,
    },
    {
      command: `npm run dev -- --host 127.0.0.1 --port ${frontendPort} --strictPort`,
      cwd: '.',
      env: { ...process.env, VITE_API_TARGET: backendOrigin },
      url: frontendOrigin,
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
})
