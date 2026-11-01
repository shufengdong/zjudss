import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getRun } from '../api/client'
import type { SimulationRun } from '../types'
import { useRunFailureDiagnosis } from './useRunFailureDiagnosis'

vi.mock('../api/client', () => ({ getRun: vi.fn() }))

const run = (id: string, analysisStatus: 'pending' | 'complete' | 'unavailable'): SimulationRun => ({
  id, sessionId: '1', modelVersionId: '3', modelContentHash: null,
  scriptResourceId: '4', scriptName: 'study.rss', scriptArtifactId: '5', scriptContentHash: null,
  scriptMessageId: null, managedWorkflowId: null, clientRunId: `client-${id}`, status: 'failed',
  stage: 'run', engineContractVersion: 'v4', agentBundleVersion: null, agentSkillRefs: [], exitCode: 7,
  error: {
    code: 'run_failed', message: `failure-${id}`, stage: 'run',
    engineLogs: [{ stream: 'stderr', text: `log-${id}`, totalBytes: 5, sentBytes: 5, truncated: false }],
    analysisStatus,
    ...(analysisStatus === 'complete' ? { analysis: {
      summary: `analysis-${id}`, facts: [], possibleCauses: [], unknowns: [], recommendations: [],
    } } : {}),
  },
  createdAt: '2026-01-01T00:00:00Z', startedAt: '2026-01-01T00:00:01Z', finishedAt: '2026-01-01T00:00:02Z',
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((accept) => { resolve = accept })
  return { promise, resolve }
}

describe('useRunFailureDiagnosis', () => {
  beforeEach(() => vi.mocked(getRun).mockReset())
  afterEach(() => vi.useRealTimers())

  it('polls_only_while_analysis_is_pending', async () => {
    vi.useFakeTimers()
    vi.mocked(getRun).mockResolvedValueOnce(run('9', 'pending')).mockResolvedValueOnce(run('9', 'complete'))

    const { result } = renderHook(() => useRunFailureDiagnosis('9', true))
    await act(async () => { await Promise.resolve() })
    expect(result.current.diagnosis?.analysisStatus).toBe('pending')
    expect(getRun).toHaveBeenCalledTimes(1)

    await act(async () => { await vi.advanceTimersByTimeAsync(749) })
    expect(getRun).toHaveBeenCalledTimes(1)
    await act(async () => { await vi.advanceTimersByTimeAsync(1) })
    expect(result.current.diagnosis?.analysisStatus).toBe('complete')
    expect(getRun).toHaveBeenCalledTimes(2)

    await act(async () => { await vi.advanceTimersByTimeAsync(1_500) })
    expect(getRun).toHaveBeenCalledTimes(2)
  })

  it('ignores_stale_detail_responses_and_clears_polling_on_unmount', async () => {
    vi.useFakeTimers()
    const first = deferred<SimulationRun>()
    const second = deferred<SimulationRun>()
    vi.mocked(getRun).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
    const { result, rerender, unmount } = renderHook(({ id }) => useRunFailureDiagnosis(id, true), {
      initialProps: { id: '1' as string | null },
    })

    rerender({ id: '2' })
    await act(async () => second.resolve(run('2', 'pending')))
    expect(result.current.diagnosis?.message).toBe('failure-2')
    await act(async () => first.resolve(run('1', 'complete')))
    expect(result.current.diagnosis?.message).toBe('failure-2')

    unmount()
    await act(async () => { vi.advanceTimersByTime(750); await Promise.resolve() })
    expect(getRun).toHaveBeenCalledTimes(2)
  })
})
