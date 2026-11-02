import { describe, expect, it } from 'vitest'
import type { Message, ScriptMessage, WorkflowProgressMessage } from '../../types'
import { projectMessageTimeline } from './timelineProjection'

const workflow = (id: string, state: string, scriptId?: string): WorkflowProgressMessage => ({
  id: `workflow-message-${id}`,
  type: 'workflow-progress',
  timestamp: '2026-11-02T00:00:00Z',
  card: {
    workflowId: id,
    state,
    executionIndex: 1,
    executionMax: 4,
    repairIndex: 0,
    repairMax: 3,
    elapsedMs: 100,
    attempts: [],
    scriptId,
    outputDatasetIds: [],
  },
})

const script = (messageId: string, resourceId: string): ScriptMessage => ({
  id: messageId,
  type: 'script',
  timestamp: '2026-11-02T00:00:01Z',
  script: {
    id: resourceId,
    sessionId: '1',
    name: `${resourceId}.rss`,
    sourceKind: 'agent',
    contentHash: `hash-${resourceId}`,
    validationStatus: 'valid',
    requiredAliases: [],
    outputs: [],
    diagnostics: {},
    preview: 'result = study();',
    previewTruncated: false,
    canRun: true,
    disabledReason: null,
    createdAt: '2026-11-02T00:00:01Z',
  },
})

describe('projectMessageTimeline', () => {
  it('groups only the matching script into a successful workflow without mutating stable order', () => {
    const successful = workflow('5', 'succeeded', '7')
    const failed = workflow('6', 'failed', '8')
    const linked = script('script-message-7', '7')
    const failedScript = script('script-message-8', '8')
    const ordinaryScript = script('script-message-9', '9')
    const messages: Message[] = [successful, failed, linked, failedScript, ordinaryScript]

    const projected = projectMessageTimeline(messages)

    expect(projected.map(item => item.message.id)).toEqual([
      'workflow-message-5',
      'workflow-message-6',
      'script-message-8',
      'script-message-9',
    ])
    expect(projected[0].linkedScript).toBe(linked)
    expect(projected[1].linkedScript).toBeUndefined()
    expect(messages.map(message => message.id)).toEqual([
      'workflow-message-5',
      'workflow-message-6',
      'script-message-7',
      'script-message-8',
      'script-message-9',
    ])
  })

  it('keeps a script standalone when its workflow is outside the loaded history window', () => {
    const orphan = script('script-message-7', '7')

    expect(projectMessageTimeline([orphan])).toEqual([{ message: orphan }])
  })
})
