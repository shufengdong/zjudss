import { act, renderHook, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { useDatasetView } from './useDatasetView'
import { getDatasetView } from '../api/client'

vi.mock('../api/client', () => ({ ApiError: class extends Error {}, getDatasetView: vi.fn() }))

it('ignores a stale view response after selecting another view', async () => {
  let resolveA!: (value: any) => void
  const a = new Promise<any>(resolve => { resolveA = resolve })
  vi.mocked(getDatasetView).mockImplementation(id => id === 'a' ? a : Promise.resolve({ viewId: 'b', datasetId: 'd', kind: 'bar', spec: {}, columns: [], rows: [], diagnostics: {} }))
  const { result, rerender } = renderHook(({ id }) => useDatasetView(id), { initialProps: { id: 'a' } })
  rerender({ id: 'b' })
  await waitFor(() => expect(result.current.data?.viewId).toBe('b'))
  await act(async () => resolveA({ viewId: 'a', datasetId: 'd', kind: 'line', spec: {}, columns: [], rows: [], diagnostics: {} }))
  expect(result.current.data?.viewId).toBe('b')
})
