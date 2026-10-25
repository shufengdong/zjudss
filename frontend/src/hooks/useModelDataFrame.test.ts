import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError, downloadModelDataFrame, getModelDataFrame, getModelDataFrameRows } from '../api/client'
import type { ModelDataFrameMetadata, ModelDataFramePage, ModelDataFrameSource } from '../types'
import { mergeModelDataFramePages, useModelDataFrame } from './useModelDataFrame'

vi.mock('../api/client', async importOriginal => {
  const original = await importOriginal<typeof import('../api/client')>()
  return { ...original, getModelDataFrame: vi.fn(), getModelDataFrameRows: vi.fn(), downloadModelDataFrame: vi.fn() }
})

const deferred = <T,>() => {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const source = (id: string, revision = 1): ModelDataFrameSource => ({
  kind: 'model-draft', dataframeId: id, draftId: `draft-${id}`, revision, tableName: 'bus',
})
const metadata = (id: string): ModelDataFrameMetadata => ({
  id, scope: 'draft', ownerId: `draft-${id}`, revision: 1, tableName: 'bus', rowCount: 3,
  sizeBytes: 10, columns: [{ name: 'id', dtype: 'Utf8', nullable: false }], schemaHash: 's', contentHash: 'c',
})
const page = (id: string, rows: unknown[][], nextCursor: string | null): ModelDataFramePage => ({
  resourceId: id, columns: ['id'], rows, nextCursor, diagnostics: {},
})

describe('useModelDataFrame', () => {
  beforeEach(() => vi.clearAllMocks())

  it('merges pages and permits only one load-more request at a time', async () => {
    vi.mocked(getModelDataFrame).mockResolvedValue(metadata('a'))
    vi.mocked(getModelDataFrameRows)
      .mockResolvedValueOnce(page('a', [['one']], 'next'))
    const more = deferred<ModelDataFramePage>()
    vi.mocked(getModelDataFrameRows).mockReturnValueOnce(more.promise)
    const { result } = renderHook(() => useModelDataFrame(source('a'), ['id']))
    await waitFor(() => expect(result.current.page?.rows).toEqual([['one']]))
    let first!: Promise<void>, second!: Promise<void>
    act(() => { first = result.current.loadMore(); second = result.current.loadMore() })
    expect(getModelDataFrameRows).toHaveBeenCalledTimes(2)
    await act(async () => { more.resolve(page('a', [['two']], null)); await Promise.all([first, second]) })
    expect(result.current.page?.rows).toEqual([['one'], ['two']])
  })

  it('rejects a page from another model resource instead of mixing rows', () => {
    const current = page('draft-dataframe:1:revision:1', [[1]], 'next')
    const next = page('draft-dataframe:1:revision:2', [[2]], null)

    expect(() => mergeModelDataFramePages(current, next)).toThrow('模型表分页资源不一致')
  })

  it('resets on source or projection changes and ignores late responses', async () => {
    const aMeta = deferred<ModelDataFrameMetadata>(), aRows = deferred<ModelDataFramePage>()
    vi.mocked(getModelDataFrame).mockImplementation(value => value.dataframeId === 'a' ? aMeta.promise : Promise.resolve(metadata('b')))
    vi.mocked(getModelDataFrameRows).mockImplementation(value => value.dataframeId === 'a' ? aRows.promise : Promise.resolve(page('b', [['b']], null)))
    const { result, rerender } = renderHook(
      ({ value, columns }) => useModelDataFrame(value, columns),
      { initialProps: { value: source('a'), columns: ['id'] } },
    )
    rerender({ value: source('b'), columns: ['id'] })
    await waitFor(() => expect(result.current.metadata?.id).toBe('b'))
    await act(async () => { aMeta.resolve(metadata('a')); aRows.resolve(page('a', [['a']], null)) })
    expect(result.current.metadata?.id).toBe('b')
    rerender({ value: source('b'), columns: [] })
    expect(result.current.page).toBeNull()
    await waitFor(() => expect(getModelDataFrameRows).toHaveBeenLastCalledWith(
      expect.objectContaining({ dataframeId: 'b' }), { limit: 100 }, expect.anything(),
    ))
  })

  it('retains loaded rows and exposes conflict after a stale draft response', async () => {
    vi.mocked(getModelDataFrame).mockResolvedValue(metadata('a'))
    vi.mocked(getModelDataFrameRows)
      .mockResolvedValueOnce(page('a', [['one']], 'next'))
      .mockRejectedValueOnce(new ApiError(409, '草稿已变化', 'draft_revision_conflict'))
    const { result } = renderHook(() => useModelDataFrame(source('a'), ['id']))
    await waitFor(() => expect(result.current.page?.rows).toEqual([['one']]))
    await act(async () => { await result.current.loadMore() })
    expect(result.current.conflict).toBe(true)
    expect(result.current.error).toBe('草稿已变化')
    expect(result.current.page?.rows).toEqual([['one']])
  })

  it('ignores an earlier reload generation and delegates download for the current source', async () => {
    const oldMeta = deferred<ModelDataFrameMetadata>(), oldRows = deferred<ModelDataFramePage>()
    vi.mocked(getModelDataFrame)
      .mockReturnValueOnce(oldMeta.promise)
      .mockResolvedValueOnce(metadata('a'))
    vi.mocked(getModelDataFrameRows)
      .mockReturnValueOnce(oldRows.promise)
      .mockResolvedValueOnce(page('a', [['new']], null))
    vi.mocked(downloadModelDataFrame).mockResolvedValue({ blob: new Blob(), filename: 'bus.parquet' })
    const current = source('a')
    const { result } = renderHook(() => useModelDataFrame(current, ['id']))
    await act(async () => { await result.current.reload() })
    await act(async () => { oldMeta.resolve(metadata('old')); oldRows.resolve(page('old', [['old']], null)) })
    expect(result.current.page?.rows).toEqual([['new']])
    await act(async () => { await result.current.download() })
    expect(downloadModelDataFrame).toHaveBeenCalledWith(current, expect.anything())
  })
})
