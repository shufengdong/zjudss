export type ScriptTurn =
  | { kind: 'text'; text: string }
  | { kind: 'tool_call'; call_id: string; name: string; args: unknown }
  | { kind: 'error'; message: string }
  | { kind: 'delay_until_released'; barrier_id: string }

export interface ScriptScenario {
  id: string
  expectedPrompt?: string | null
  turns: ScriptTurn[]
}

const backend = 'http://127.0.0.1:18080'
const token = 'e2e-control-token'

export class ControlClient {
  private async call(path: string, init: RequestInit = {}) {
    const response = await fetch(`${backend}${path}`, {
      ...init,
      headers: { 'x-zjudss-e2e-token': token, 'content-type': 'application/json', ...init.headers },
    })
    if (!response.ok) throw new Error(`control ${path} failed: ${response.status} ${await response.text()}`)
    return response.status === 204 ? null : response.json()
  }

  enqueueLlm(scenario: ScriptScenario) {
    return this.call('/__e2e/llm-scenarios', { method: 'POST', body: JSON.stringify(scenario) })
  }

  enqueueEngine(scenario: unknown) {
    return this.call('/__e2e/engine-scenarios', { method: 'POST', body: JSON.stringify(scenario) })
  }

  release(id: string) {
    return this.call(`/__e2e/barriers/${encodeURIComponent(id)}/release`, { method: 'POST' })
  }

  injectFault(point: 'event_commit') {
    return this.call('/__e2e/faults', { method: 'POST', body: JSON.stringify({ point }) })
  }

  setStorageLimits(scope: { kind: string; key: string }, limits: { artifactLimit: number; byteLimit: number; warningRatio: number }) {
    return this.call('/__e2e/storage-limits', { method: 'POST', body: JSON.stringify({ ...scope, ...limits }) })
  }

  seedDataResource(input: { userId: string; sessionId: number; uploadBatchId: number; resourceId: string }) {
    return this.call('/__e2e/data-resources', { method: 'POST', body: JSON.stringify(input) })
  }

  dropHistoryIndex() {
    return this.call('/__e2e/history-index/drop', { method: 'POST' })
  }

  rebuildHistoryIndex() {
    return this.call('/__e2e/history-index/rebuild', { method: 'POST' })
  }

  trace() {
    return this.call('/__e2e/trace')
  }
}
