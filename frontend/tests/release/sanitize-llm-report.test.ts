import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { describe, expect, test } from 'vitest'

const repoRoot = path.resolve(import.meta.dirname, '../../..')
const sanitizer = path.join(repoRoot, 'scripts/sanitize-release-llm-report.mjs')

describe('release LLM report sanitizer', () => {
  test('redacts every provider credential from a failed report', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'zjudss-llm-report-'))
    const reportPath = path.join(directory, 'llm-report.json')
    try {
      fs.writeFileSync(reportPath, JSON.stringify({
        success: false,
        attempts: [{ reason: 'deepseek-secret and bailian-secret' }],
        nested: { diagnostic: 'prefix bailian-secret suffix' },
      }))
      execFileSync('node', [sanitizer], {
        env: {
          ...process.env,
          RELEASE_LLM_REPORT: reportPath,
          DEEPSEEK_API_KEY: 'deepseek-secret',
          BAILIAN_API_KEY: '  bailian-secret  ',
        },
      })
      const text = fs.readFileSync(reportPath, 'utf8')
      expect(text).not.toContain('deepseek-secret')
      expect(text).not.toContain('bailian-secret')
      expect(JSON.parse(text).attempts[0].reason).toBe('<redacted> and <redacted>')
      expect(JSON.parse(text).nested.diagnostic).toBe('prefix <redacted> suffix')
    } finally {
      fs.rmSync(directory, { recursive: true, force: true })
    }
  })

  test('removes malformed evidence instead of leaving an uploadable secret', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'zjudss-llm-report-'))
    const reportPath = path.join(directory, 'llm-report.json')
    try {
      fs.writeFileSync(reportPath, '{"secret":"bailian-secret"')
      expect(() => execFileSync('node', [sanitizer], {
        env: {
          ...process.env,
          RELEASE_LLM_REPORT: reportPath,
          BAILIAN_API_KEY: 'bailian-secret',
        },
        stdio: 'pipe',
      })).toThrow()
      expect(fs.existsSync(reportPath)).toBe(false)
    } finally {
      fs.rmSync(directory, { recursive: true, force: true })
    }
  })
})
