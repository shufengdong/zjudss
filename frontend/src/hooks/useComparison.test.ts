import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getComparison } from '../api/client'
import type { RunComparison } from '../types'
import { useComparison } from './useComparison'

vi.mock('../api/client', () => ({ getComparison: vi.fn() }))

const deferred = <T,>() => {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

const comparison = (id: string): RunComparison => ({
  id,
  sessionId: 'session',
  baselineRunId: `baseline-${id}`,
  candidateRunId: `candidate-${id}`,
  compatibility: {},
  analysisRunId: `analysis-${id}`,
  createdAt: '2026-11-05T00:00:00Z',
  datasets: [],
})

describe('useComparison', () => {
  beforeEach(() => vi.clearAllMocks())

  it('ignores a previous comparison response after the id changes', async () => {
    const oldRequest = deferred<RunComparison>()
    const currentRequest = deferred<RunComparison>()
    vi.mocked(getComparison).mockImplementation(id => id === 'old' ? oldRequest.promise : currentRequest.promise)
    const { result, rerender } = renderHook(({ id }) => useComparison(id), {
      initialProps: { id: 'old' as string | null },
    })

    rerender({ id: 'current' })
    await act(async () => currentRequest.resolve(comparison('current')))
    await waitFor(() => expect(result.current.value?.id).toBe('current'))
    await act(async () => oldRequest.resolve(comparison('old')))

    expect(result.current.value?.id).toBe('current')
  })

  it('exposes a retryable error without retaining stale data', async () => {
    vi.mocked(getComparison)
      .mockRejectedValueOnce(new Error('比较暂不可用'))
      .mockResolvedValueOnce(comparison('retry'))
    const { result } = renderHook(() => useComparison('retry'))

    await waitFor(() => expect(result.current.error).toBe('比较暂不可用'))
    expect(result.current.value).toBeNull()
    expect(result.current.loading).toBe(false)
    await act(async () => result.current.reload())

    expect(result.current.value?.id).toBe('retry')
    expect(result.current.error).toBeNull()
  })
})
