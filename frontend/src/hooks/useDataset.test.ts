import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useDataset } from './useDataset'
import { getDataset, getDatasetRows } from '../api/client'

vi.mock('../api/client', () => ({ getDataset: vi.fn(), getDatasetRows: vi.fn() }))

const deferred = <T,>() => {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const metadata = (id: string) => ({ id, taskId: 't', analysisRunId: null, name: id, rowCount: 2, sizeBytes: 1, columns: [{ name: 'key', dtype: 'Utf8', nullable: false }], views: [] })
const page = (id: string, rows: unknown[][], nextCursor: string | null) => ({ datasetId: id, columns: ['key'], rows, nextCursor, diagnostics: {} })

describe('useDataset request ordering', () => {
  beforeEach(() => vi.clearAllMocks())

  it('ignores dataset A responses that arrive after switching to B', async () => {
    const aMeta = deferred<any>(), aRows = deferred<any>(), bMeta = deferred<any>(), bRows = deferred<any>()
    vi.mocked(getDataset).mockImplementation(id => (id === 'a' ? aMeta.promise : bMeta.promise))
    vi.mocked(getDatasetRows).mockImplementation(id => (id === 'a' ? aRows.promise : bRows.promise))
    const { result, rerender } = renderHook(({ id }) => useDataset(id), { initialProps: { id: 'a' } })
    rerender({ id: 'b' })
    await act(async () => { bMeta.resolve(metadata('b')); bRows.resolve(page('b', [['b']], null)) })
    await waitFor(() => expect(result.current.metadata?.id).toBe('b'))
    await act(async () => { aMeta.resolve(metadata('a')); aRows.resolve(page('a', [['a']], null)) })
    expect(result.current.metadata?.id).toBe('b')
    expect(result.current.page?.datasetId).toBe('b')
  })

  it('keeps loaded rows after loadMore failure and allows retry', async () => {
    vi.mocked(getDataset).mockResolvedValue(metadata('a'))
    vi.mocked(getDatasetRows)
      .mockResolvedValueOnce(page('a', [['first']], 'next'))
      .mockRejectedValueOnce(new Error('下一页失败'))
      .mockResolvedValueOnce(page('a', [['second']], null))
    const { result } = renderHook(() => useDataset('a'))
    await waitFor(() => expect(result.current.page?.rows).toEqual([['first']]))
    await act(async () => { await result.current.loadMore() })
    expect(result.current.error).toBe('下一页失败')
    expect(result.current.page?.rows).toEqual([['first']])
    await act(async () => { await result.current.loadMore() })
    expect(result.current.error).toBeNull()
    expect(result.current.page?.rows).toEqual([['first'], ['second']])
  })

  it('reloads the projection and keeps it for later pages', async () => {
    vi.mocked(getDataset).mockResolvedValue(metadata('a'))
    vi.mocked(getDatasetRows)
      .mockResolvedValueOnce(page('a', [['first']], null))
      .mockResolvedValueOnce({ ...page('a', [[1]], 'next'), columns: ['value'] })
      .mockResolvedValueOnce({ ...page('a', [[2]], null), columns: ['value'] })
    const { result, rerender } = renderHook(({ columns }) => useDataset('a', columns), {
      initialProps: { columns: undefined as string[] | undefined },
    })
    await waitFor(() => expect(result.current.page?.rows).toEqual([['first']]))
    rerender({ columns: ['value'] })
    await waitFor(() => expect(result.current.page?.columns).toEqual(['value']))
    await act(async () => { await result.current.loadMore() })
    expect(getDatasetRows).toHaveBeenLastCalledWith('a', { limit: 100, cursor: 'next', columns: 'value' })
  })
})
