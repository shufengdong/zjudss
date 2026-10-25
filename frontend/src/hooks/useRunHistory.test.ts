import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { listRuns } from '../api/client'
import type { SimulationRun } from '../types'
import { useRunHistory } from './useRunHistory'

vi.mock('../api/client', () => ({ listRuns: vi.fn() }))

const mockedListRuns = vi.mocked(listRuns)
const run = (id: string): SimulationRun => ({
  id, sessionId: '1', modelVersionId: '3', modelContentHash: 'm'.repeat(64),
  scriptResourceId: '4', scriptName: 'flow.rss', scriptArtifactId: '5',
  scriptContentHash: 's'.repeat(64), clientRunId: `client-${id}`, status: 'succeeded',
  stage: 'completed', engineContractVersion: 'v3', agentBundleVersion: null,
  agentSkillRefs: [], exitCode: 0, error: null, createdAt: '2026-01-01T00:00:00Z',
  startedAt: '2026-01-01T00:00:01Z', finishedAt: '2026-01-01T00:00:02Z',
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((accept, decline) => { resolve = accept; reject = decline })
  return { promise, resolve, reject }
}

describe('useRunHistory', () => {
  beforeEach(() => mockedListRuns.mockReset())

  it('exposes loading and ready states', async () => {
    const request = deferred<SimulationRun[]>()
    mockedListRuns.mockReturnValue(request.promise)
    const { result } = renderHook(() => useRunHistory('1'))
    expect(result.current.status).toBe('loading')
    act(() => request.resolve([run('9')]))
    await waitFor(() => expect(result.current.status).toBe('ready'))
    expect(result.current.runs.map(({ id }) => id)).toEqual(['9'])
    expect(result.current.error).toBeNull()
  })

  it('exposes a useful error and retries on demand', async () => {
    mockedListRuns.mockRejectedValueOnce(new Error('网络中断')).mockResolvedValueOnce([run('10')])
    const { result } = renderHook(() => useRunHistory('1'))
    await waitFor(() => expect(result.current.status).toBe('error'))
    expect(result.current.error).toBe('网络中断')
    act(() => result.current.reload())
    expect(result.current.status).toBe('loading')
    await waitFor(() => expect(result.current.status).toBe('ready'))
    expect(result.current.runs[0].id).toBe('10')
  })

  it('ignores responses from an obsolete session', async () => {
    const first = deferred<SimulationRun[]>()
    const second = deferred<SimulationRun[]>()
    mockedListRuns.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
    const { result, rerender } = renderHook(({ id }) => useRunHistory(id), {
      initialProps: { id: '1' as string | null },
    })
    rerender({ id: '2' })
    act(() => second.resolve([run('2')]))
    await waitFor(() => expect(result.current.runs[0]?.id).toBe('2'))
    act(() => first.resolve([run('1')]))
    await act(async () => { await first.promise })
    expect(result.current.runs[0]?.id).toBe('2')
  })

  it('returns an idle empty state when no session is selected', () => {
    const { result } = renderHook(() => useRunHistory(null))
    expect(result.current).toMatchObject({ runs: [], status: 'idle', error: null })
    expect(mockedListRuns).not.toHaveBeenCalled()
  })
})
