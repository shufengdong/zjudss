import { directApi } from './server'

export async function readEvent(
  path: string,
  identity: { subject: string; tenant?: string },
  lastEventId?: string,
  expectedEvent = 'message',
) {
  const controller = new AbortController()
  const response = await directApi(
    path,
    { ...identity, token: 'e2e-proxy-token' },
    { headers: lastEventId ? { 'last-event-id': lastEventId } : {}, signal: controller.signal },
  )
  if (!response.ok || !response.body) throw new Error(`SSE failed: ${response.status}`)
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let content = ''
  try {
    while (!content.includes(`event: ${expectedEvent}`)) {
      const item = await reader.read()
      if (item.done) break
      content += decoder.decode(item.value, { stream: true })
    }
  } finally {
    controller.abort()
    await reader.cancel().catch(() => undefined)
  }
  return content
}
