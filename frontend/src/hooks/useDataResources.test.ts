import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { listDataResources } from '../api/client'
import { useDataResources } from './useDataResources'

vi.mock('../api/client', () => ({ listDataResources: vi.fn() }))

const resource = (id: string, sessionId: string) => ({
  id, sessionId, displayName: id, alias: id, status: 'live', declaredSchemaRef: null,
  sourceKind: 'upload', schema: [], schemaHash: `schema-${id}`, contentHash: `hash-${id}`,
  rowCount: 1, sizeBytes: 10, originalAvailable: false, sourceBatchId: null,
  sourceModelVersionId: null, replacedByResourceId: null, lineage: {}, createdAtMs: 1,
})

describe('useDataResources', () => {
  beforeEach(() => vi.clearAllMocks())

  it('isolates resources by session and ignores a late previous response', async () => {
    let release!: (value: Awaited<ReturnType<typeof listDataResources>>) => void
    vi.mocked(listDataResources)
      .mockImplementationOnce(() => new Promise(resolve => { release = resolve }))
      .mockResolvedValueOnce({ resources: [resource('next', 's2')], nextCursor: null })
    const { result, rerender } = renderHook(({ sessionId }) => useDataResources(sessionId), {
      initialProps: { sessionId: 's1' as string | null },
    })
    rerender({ sessionId: 's2' })
    await waitFor(() => expect(result.current.resources.map(value => value.id)).toEqual(['next']))
    await act(async () => release({ resources: [resource('old', 's1')], nextCursor: null }))
    expect(result.current.resources.map(value => value.id)).toEqual(['next'])
  })

  it('appends cursor pages without duplicating resources', async () => {
    vi.mocked(listDataResources)
      .mockResolvedValueOnce({ resources: [resource('a', 's')], nextCursor: 'cursor-2' })
      .mockResolvedValueOnce({ resources: [resource('a', 's'), resource('b', 's')], nextCursor: null })
    const { result } = renderHook(() => useDataResources('s'))
    await waitFor(() => expect(result.current.resources).toHaveLength(1))
    await act(async () => result.current.loadMore())
    expect(listDataResources).toHaveBeenLastCalledWith('s', { cursor: 'cursor-2', limit: 50 })
    expect(result.current.resources.map(value => value.id)).toEqual(['a', 'b'])
  })
})
