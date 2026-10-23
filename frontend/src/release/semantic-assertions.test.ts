import { describe, expect, test } from 'vitest'
import {
  expectToolTrace,
  type ToolTraceEntry,
} from '../../../integration-tests/release/semantic-assertions'

const call = (toolName: string, status: ToolTraceEntry['status'], args: Record<string, unknown> = {}): ToolTraceEntry => ({
  callId: `${toolName}-${status}`, toolName, status, args,
})

describe('release semantic assertions', () => {
  test('accepts stable IDs and skills loaded before gated tools', () => {
    expect(() => expectToolTrace([
      call('load_agent_skill', 'success', { skill_id: 'study-planning' }),
      call('load_agent_skill', 'success', { skill_id: 'rss-generation' }),
      call('run_study', 'running', { modelVersionId: 7 }),
      call('run_study', 'success', { modelVersionId: 7 }),
    ], {
      requiredTools: ['run_study'],
      gatedTools: { run_study: ['study-planning', 'rss-generation'] },
    })).not.toThrow()
  })

  test('rejects illegal tools, unstable IDs, failed tools, and missing skill ordering', () => {
    expect(() => expectToolTrace([call('shell', 'success')], { requiredTools: ['shell'] })).toThrow(/disallowed/)
    expect(() => expectToolTrace([call('inspect_example', 'success', { exampleId: '/tmp/case.rss' })], { requiredTools: ['inspect_example'] })).toThrow(/stable example ID/)
    expect(() => expectToolTrace([call('run_study', 'failed', { modelVersionId: 1 })], { requiredTools: ['run_study'] })).toThrow(/did not succeed/)
    expect(() => expectToolTrace([
      call('run_study', 'running', { modelVersionId: 1 }),
      call('load_agent_skill', 'success', { skill_id: 'study-planning' }),
      call('run_study', 'success', { modelVersionId: 1 }),
    ], { requiredTools: ['run_study'], gatedTools: { run_study: ['study-planning'] } })).toThrow(/not loaded before/)
  })
})
