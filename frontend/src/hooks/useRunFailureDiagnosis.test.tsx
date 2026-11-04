import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getRun } from '../api/client'
import { failureDiagnosis, simulationRun } from '../test/fixtures'
import type { SimulationRun } from '../types'
import { useRunFailureDiagnosis } from './useRunFailureDiagnosis'

vi.mock('../api/client', () => ({ getRun: vi.fn() }))

const failedRun = (
  id: string,
  analysisStatus: 'pending' | 'complete' | 'unavailable',
): SimulationRun => simulationRun({
  id,
  clientRunId: `client-${id}`,
  status: 'failed',
  stage: 'run',
  exitCode: 7,
  error: failureDiagnosis({
    message: `failure-${id}`,
    engineLogs: [
      { stream: 'stderr', text: `log-${id}`, totalBytes: 5, sentBytes: 5, truncated: false },
    ],
    analysisStatus,
    analysis: analysisStatus === 'complete' ? {
      summary: `analysis-${id}`, facts: [], possibleCauses: [], unknowns: [], recommendations: [],
    } : null,
  }),
  createdAt: '2026-01-01T00:00:00Z',
  startedAt: '2026-01-01T00:00:01Z',
  finishedAt: '2026-01-01T00:00:02Z',
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
    vi.mocked(getRun).mockResolvedValueOnce(failedRun('9', 'pending')).mockResolvedValueOnce(failedRun('9', 'complete'))

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
    await act(async () => second.resolve(failedRun('2', 'pending')))
    expect(result.current.diagnosis?.message).toBe('failure-2')
    await act(async () => first.resolve(failedRun('1', 'complete')))
    expect(result.current.diagnosis?.message).toBe('failure-2')

    unmount()
    await act(async () => { vi.advanceTimersByTime(750); await Promise.resolve() })
    expect(getRun).toHaveBeenCalledTimes(2)
  })
})
