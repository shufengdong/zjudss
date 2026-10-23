import { describe, expect, test } from 'vitest'
import {
  agentTurnCompleted,
  expectToolTrace,
  readAllMessages,
  type ToolTraceEntry,
} from '../../../integration-tests/release/semantic-assertions'

const call = (toolName: string, status: ToolTraceEntry['status'], args: Record<string, unknown> = {}): ToolTraceEntry => ({
  callId: `${toolName}-${status}`, toolName, status, args,
})

describe('release semantic assertions', () => {
  test('waits for an assistant response after the final tool result', () => {
    expect(agentTurnCompleted([
      { id: 1, type: 'assistant' },
      { id: 2, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'running' },
      { id: 3, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'success' },
      { id: 4, type: 'assistant' },
    ], ['confirm_dataframe_mapping'])).toBe(false)
    expect(agentTurnCompleted([
      { id: 1, type: 'assistant' },
      { id: 2, type: 'tool-call', callId: 'inspect', toolName: 'inspect_workspace', status: 'success' },
      { id: 3, type: 'tool-call', callId: 'confirm', toolName: 'confirm_dataframe_mapping', status: 'success' },
      { id: 4, type: 'assistant' },
    ], ['confirm_dataframe_mapping'])).toBe(true)
  })

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

  test('accepts a failed tool call only when the agent recovers with the same tool', () => {
    expect(() => expectToolTrace([
      call('run_result_analysis', 'running', { taskId: 1 }),
      call('run_result_analysis', 'failed', { taskId: 1 }),
      call('run_result_analysis', 'running', { taskId: 1 }),
      call('run_result_analysis', 'success', { taskId: 1 }),
    ], { requiredTools: ['run_result_analysis'] })).not.toThrow()
    expect(() => expectToolTrace([
      call('run_result_analysis', 'failed', { taskId: 1 }),
    ], { requiredTools: [] })).toThrow(/failed without recovery/)
  })

  test('paginates the complete message history with nextBeforeId', async () => {
    const paths: string[] = []
    const api = {
      async get(path: string) {
        paths.push(path)
        const second = path.includes('beforeId=101')
        return {
          ok: () => true,
          status: () => 200,
          json: async () => second
            ? { messages: [{ id: '1' }], hasMoreBefore: false, nextBeforeId: '1' }
            : { messages: [{ id: '101' }], hasMoreBefore: true, nextBeforeId: '101' },
        }
      },
    }

    await expect(readAllMessages(api, '7')).resolves.toEqual([{ id: '101' }, { id: '1' }])
    expect(paths).toEqual([
      '/api/sessions/7/messages?limit=100',
      '/api/sessions/7/messages?limit=100&beforeId=101',
    ])
  })
})
