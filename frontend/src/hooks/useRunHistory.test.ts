import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { cancelManagedWorkflow, cancelRun, listRuns } from '../api/client'
import { simulationRun } from '../test/fixtures'
import type { SimulationRun } from '../types'
import { useRunHistory } from './useRunHistory'

vi.mock('../api/client', () => ({
  cancelManagedWorkflow: vi.fn(),
  cancelRun: vi.fn(),
  deleteRun: vi.fn(),
  listRuns: vi.fn(),
  previewRunDeletion: vi.fn(),
}))

const mockedListRuns = vi.mocked(listRuns)
const mockedCancelRun = vi.mocked(cancelRun)
const mockedCancelManagedWorkflow = vi.mocked(cancelManagedWorkflow)
const historyItem = (id: string) => simulationRun({
  id,
  clientRunId: `client-${id}`,
  scriptName: 'flow.rss',
  createdAt: '2026-01-01T00:00:00Z',
  startedAt: '2026-01-01T00:00:01Z',
  finishedAt: '2026-01-01T00:00:02Z',
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((accept, decline) => { resolve = accept; reject = decline })
  return { promise, resolve, reject }
}

describe('useRunHistory', () => {
  beforeEach(() => {
    mockedListRuns.mockReset()
    mockedCancelRun.mockReset()
    mockedCancelManagedWorkflow.mockReset()
  })

  it('exposes loading and ready states', async () => {
    const request = deferred<SimulationRun[]>()
    mockedListRuns.mockReturnValue(request.promise)
    const { result } = renderHook(() => useRunHistory('1'))
    expect(result.current.status).toBe('loading')
    act(() => request.resolve([historyItem('9')]))
    await waitFor(() => expect(result.current.status).toBe('ready'))
    expect(result.current.runs.map(({ id }) => id)).toEqual(['9'])
    expect(result.current.error).toBeNull()
  })

  it('exposes a useful error and retries on demand', async () => {
    mockedListRuns.mockRejectedValueOnce(new Error('网络中断')).mockResolvedValueOnce([historyItem('10')])
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
    act(() => second.resolve([historyItem('2')]))
    await waitFor(() => expect(result.current.runs[0]?.id).toBe('2'))
    act(() => first.resolve([historyItem('1')]))
    await act(async () => { await first.promise })
    expect(result.current.runs[0]?.id).toBe('2')
  })

  it('returns an idle empty state when no session is selected', () => {
    const { result } = renderHook(() => useRunHistory(null))
    expect(result.current).toMatchObject({ runs: [], status: 'idle', error: null })
    expect(mockedListRuns).not.toHaveBeenCalled()
  })

  it('routes ordinary and managed cancellation to their exclusive endpoints and reloads after success', async () => {
    const ordinary = historyItem('11')
    const managed = historyItem('12')
    ordinary.status = 'running'
    ordinary.finishedAt = null
    managed.status = 'queued'
    managed.startedAt = null
    managed.finishedAt = null
    managed.managedWorkflowId = 'workflow-12'
    const refreshedOrdinary = { ...ordinary, status: 'failed' as const, stage: 'cancelled_by_user' }
    const refreshedManaged = { ...managed, status: 'failed' as const, stage: 'cancelled_by_user' }
    mockedListRuns
      .mockResolvedValueOnce([ordinary, managed])
      .mockResolvedValueOnce([refreshedOrdinary, managed])
      .mockResolvedValueOnce([refreshedOrdinary, refreshedManaged])
    mockedCancelRun.mockResolvedValue(refreshedOrdinary)
    mockedCancelManagedWorkflow.mockResolvedValue({} as Awaited<ReturnType<typeof cancelManagedWorkflow>>)

    const { result } = renderHook(() => useRunHistory('1'))
    await waitFor(() => expect(result.current.status).toBe('ready'))

    await act(async () => result.current.cancel(ordinary))
    expect(mockedCancelRun).toHaveBeenCalledWith('11')
    expect(mockedCancelManagedWorkflow).not.toHaveBeenCalled()
    await waitFor(() => expect(result.current.runs[0].status).toBe('failed'))

    await act(async () => result.current.cancel(managed))
    expect(mockedCancelManagedWorkflow).toHaveBeenCalledWith('workflow-12')
    expect(mockedCancelRun).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(result.current.runs[1].status).toBe('failed'))
  })

  it('keeps the current row and propagates cancellation failures without reloading', async () => {
    const running = historyItem('13')
    running.status = 'running'
    running.finishedAt = null
    mockedListRuns.mockResolvedValue([running])
    mockedCancelRun.mockRejectedValue(new Error('取消请求失败'))

    const { result } = renderHook(() => useRunHistory('1'))
    await waitFor(() => expect(result.current.status).toBe('ready'))

    await expect(act(async () => result.current.cancel(running))).rejects.toThrow('取消请求失败')
    expect(result.current.runs).toEqual([running])
    expect(mockedListRuns).toHaveBeenCalledTimes(1)
  })
})
