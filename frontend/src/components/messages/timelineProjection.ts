import type { Message, ScriptMessage } from '../../types'

export interface TimelineItem {
  message: Message
  linkedScript?: ScriptMessage
}

export function projectMessageTimeline(messages: Message[]): TimelineItem[] {
  const scriptsByResourceId = new Map<string, { index: number; message: ScriptMessage }>()
  messages.forEach((message, index) => {
    if (message.type === 'script' && !scriptsByResourceId.has(message.script.id)) {
      scriptsByResourceId.set(message.script.id, { index, message })
    }
  })

  const linkedByWorkflowMessageId = new Map<string, ScriptMessage>()
  const linkedScriptMessageIds = new Set<string>()
  messages.forEach((message, index) => {
    if (message.type !== 'workflow-progress' || message.card.state !== 'succeeded' || !message.card.scriptId) return
    const candidate = scriptsByResourceId.get(message.card.scriptId)
    if (!candidate || candidate.index <= index || linkedScriptMessageIds.has(candidate.message.id)) return
    linkedByWorkflowMessageId.set(message.id, candidate.message)
    linkedScriptMessageIds.add(candidate.message.id)
  })

  return messages.flatMap((message) => {
    if (message.type === 'script' && linkedScriptMessageIds.has(message.id)) return []
    const linkedScript = linkedByWorkflowMessageId.get(message.id)
    return [{ message, ...(linkedScript ? { linkedScript } : {}) }]
  })
}
