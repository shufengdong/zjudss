import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { getChartViewData } from '../api/client'
import type { BarChartModel, LineChartModel } from '../types'
import { useChartView } from './useChartView'

vi.mock('../api/client', () => ({ ApiError: class extends Error {}, getChartViewData: vi.fn() }))

const base = {
  contractVersion: 'chart-model-v1' as const,
  sourceDatasetId: 'd', sourceDeleted: false, name: 'view', title: null,
  configHash: 'hash',
  diagnostics: { inputRows: 1, candidatePoints: 1, outputPoints: 1, droppedMissing: 0,
    droppedNonFinite: 0, missingComponents: 0, aggregatedGroups: 0, sampledPoints: 0 },
}
const presentation = {
  legend: { show: true, position: 'auto' as const, orientation: 'auto' as const },
  palette: 'colorblind' as const, title: { show: false }, axis: { showGrid: true },
}
const line: LineChartModel = {
  ...base, kind: 'line', viewId: 'a', order: 'source', aggregation: 'none', presentation: {
    ...presentation, line: { width: 2, style: 'solid', showSymbol: false },
  },
  xAxis: { type: 'value', name: 'x', unit: null }, yAxis: { name: 'y', unit: null },
  series: [{ name: 'y', points: [{ x: 1, y: 2, ordinal: 0 }] }],
}
const bar: BarChartModel = {
  ...base, kind: 'bar', viewId: 'b', presentation,
  xAxis: { type: 'category', name: 'x', unit: null }, yAxis: { name: 'y', unit: null },
  categories: [{ kind: 'integer', value: '9007199254740993' }],
  series: [{ name: 'y', values: [2] }],
}

afterEach(() => vi.clearAllMocks())

it('ignores a stale chart response and aborts it after selecting another view', async () => {
  let resolveA!: (value: LineChartModel) => void
  const a = new Promise<LineChartModel>(resolve => { resolveA = resolve })
  vi.mocked(getChartViewData).mockImplementation(id => id === 'a' ? a : Promise.resolve(bar))
  const { result, rerender } = renderHook(({ id }) => useChartView(id), { initialProps: { id: 'a' } })
  rerender({ id: 'b' })
  await waitFor(() => expect(result.current.data?.viewId).toBe('b'))
  await act(async () => resolveA(line))
  expect(result.current.data?.viewId).toBe('b')
  expect(vi.mocked(getChartViewData).mock.calls[0][1]).toBeInstanceOf(AbortSignal)
  expect(vi.mocked(getChartViewData).mock.calls[0][1]?.aborted).toBe(true)
})
