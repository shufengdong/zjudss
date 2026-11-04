import { describe, expect, it } from 'vitest'
import type { ChartModel, ChartPalette, LineChartModel } from '../../types'
import { chartAriaLabel, compileView } from './ChartRenderer'
import { CHART_PALETTES } from './charts/shared'

const diagnostics = { inputRows: 4, candidatePoints: 4, outputPoints: 4, droppedMissing: 0,
  droppedNonFinite: 0, missingComponents: 0, aggregatedGroups: 0, sampledPoints: 0 }
const base = { contractVersion: 'chart-model-v1' as const, viewId: 'v', sourceDatasetId: 'd',
  sourceDeleted: false, name: 'view', title: '标题', configHash: 'hash', diagnostics }
const presentation = (palette: ChartPalette = 'colorblind') => ({
  legend: { show: true, position: 'right' as const, orientation: 'vertical' as const },
  palette, title: { show: true }, axis: { showGrid: true },
})
const models: ChartModel[] = [
  { ...base, kind: 'line', order: 'source', aggregation: 'none', presentation: { ...presentation(), line: { width: 3 as const, style: 'dashed' as const, showSymbol: true } },
    xAxis: { type: 'category', name: 'bus', unit: null }, yAxis: { name: 'load', unit: 'MW' },
    series: [{ name: 'load', points: [{ x: { kind: 'integer', value: '9007199254740993' }, y: 2, ordinal: 0 }] }] },
  { ...base, kind: 'bar', presentation: presentation(), xAxis: { type: 'category', name: 'bus', unit: null },
    yAxis: { name: 'load', unit: 'MW' }, categories: [{ kind: 'boolean', value: true }],
    series: [{ name: 'load', values: [2] }] },
  { ...base, kind: 'scatter', presentation: presentation(), xAxis: { type: 'value', name: 'p', unit: 'MW' },
    yAxis: { name: 'q', unit: 'MVar' }, series: [{ name: 'q', points: [{ x: 1, y: 2, ordinal: 0 }] }] },
  { ...base, kind: 'heatmap', presentation: presentation(), xAxis: { name: 'bus', categories: [{ kind: 'string', value: 'A' }] },
    yAxis: { name: 'phase', categories: [{ kind: 'integer', value: '1' }] },
    xCategories: [{ kind: 'string', value: 'A' }], yCategories: [{ kind: 'integer', value: '1' }],
    value: { name: 'voltage', unit: 'p.u.', min: .9, max: 1.1 }, cells: [{ xIndex: 0, yIndex: 0, value: 1 }] },
  { ...base, kind: 'boxplot', presentation: presentation(),
    categoryAxis: { name: 'bus', categories: [{ kind: 'string', value: 'A' }] }, valueAxis: { name: 'voltage', unit: 'p.u.' },
    series: [{ name: 'voltage', values: [[.9, .95, 1, 1.05, 1.1]], outliers: [{ categoryIndex: 0, value: 1.2 }] }] },
  { ...base, kind: 'histogram', presentation: presentation(), value: { name: 'voltage', unit: 'p.u.' },
    bins: [{ start: .9, end: 1, count: 3, includesEnd: false }, { start: 1, end: 1.1, count: 2, includesEnd: true }] },
]

const unsafeKey = (value: unknown): boolean => !!value && typeof value === 'object' &&
  Object.entries(value).some(([key, child]) => key.toLowerCase().includes('formatter') || unsafeKey(child))

describe('typed controlled ECharts builders', () => {
  for (const model of models) it(`builds ${model.kind} exclusively from ChartModel`, () => {
    const option = compileView(model)
    expect(option.series).toBeTruthy()
    expect(option.animation).toBe(false)
    expect(option.color).toEqual(CHART_PALETTES.colorblind)
    expect(unsafeKey(option)).toBe(false)
  })

  it('preserves tagged integer categories and server materialized histogram bins', () => {
    const line = compileView(models[0])
    expect((line.xAxis as { data: string[] }).data).toEqual(['9007199254740993'])
    const histogram = compileView(models[5])
    expect((histogram.series as { data: number[] }[])[0].data).toEqual([3, 2])
    expect((histogram.xAxis as { data: string[] }).data).toEqual(['[0.9, 1)', '[1, 1.1]'])
  })

  it('aligns sparse category-line series without shifting values to another category', () => {
    const model = { ...models[0], xAxis: { type: 'category' as const, name: 'bus', unit: null }, series: [
      { name: 'p', points: [{ x: { kind: 'string' as const, value: 'B' }, y: 2, ordinal: 1 }] },
      { name: 'q', points: [{ x: { kind: 'string' as const, value: 'A' }, y: 1, ordinal: 0 }] },
    ] } as LineChartModel
    const option = compileView(model)
    expect((option.xAxis as { data: string[] }).data).toEqual(['A', 'B'])
    expect((option.series as { data: (number | null)[] }[]).map(series => series.data)).toEqual([[null, 2], [1, null]])
    const descending = compileView({ ...model, order: 'xDesc' })
    expect((descending.xAxis as { data: string[] }).data).toEqual(['B', 'A'])
    expect((descending.series as { data: (number | null)[] }[]).map(series => series.data)).toEqual([[2, null], [null, 1]])
  })

  it('keeps repeated unaggregated categories in distinct ordinal slots but aligns aggregated categories', () => {
    const model = { ...models[0], xAxis: { type: 'category' as const, name: 'bus', unit: null }, series: [
      { name: 'p', points: [{ x: { kind: 'string' as const, value: 'A' }, y: 2, ordinal: 1 }] },
      { name: 'q', points: [{ x: { kind: 'string' as const, value: 'A' }, y: 1, ordinal: 0 }] },
    ] } as LineChartModel
    const unaggregated = compileView(model)
    expect((unaggregated.xAxis as { data: string[] }).data).toEqual(['A', 'A'])
    expect((unaggregated.series as { data: (number | null)[] }[]).map(series => series.data))
      .toEqual([[null, 2], [1, null]])

    const aggregated = compileView({ ...model, aggregation: 'mean' })
    expect((aggregated.xAxis as { data: string[] }).data).toEqual(['A'])
    expect((aggregated.series as { data: (number | null)[] }[]).map(series => series.data)).toEqual([[2], [1]])
  })

  it('keeps negative and positive zero as distinct numeric category slots', () => {
    const model = { ...models[0], order: 'xAsc' as const, xAxis: { type: 'category' as const, name: 'x', unit: null }, series: [{
      name: 'y', points: [
        { x: { kind: 'number' as const, value: 0 }, y: 2, ordinal: 1 },
        { x: { kind: 'number' as const, value: -0 }, y: 1, ordinal: 0 },
      ],
    }] } as LineChartModel
    expect((compileView(model).series as { data: number[] }[])[0].data).toEqual([1, 2])
  })

  it('compiles missing box values to ECharts gaps and applies palette to heatmap scale', () => {
    const boxplot = { ...models[4], series: [{ name: 'voltage', values: [null], outliers: [] }] } as ChartModel
    expect((compileView(boxplot).series as { data: unknown[] }[])[0].data).toEqual(['-'])
    expect((compileView(models[3]).visualMap as { inRange: { color: string[] } }).inRange.color)
      .toEqual(CHART_PALETTES.colorblind)
  })

  it('freezes all four palettes and assigns stable series colors', () => {
    expect(CHART_PALETTES).toEqual({
      default: ['#5470C6','#91CC75','#FAC858','#EE6666','#73C0DE','#3BA272','#FC8452','#9A60B4','#EA7CCC'],
      colorblind: ['#0072B2','#E69F00','#009E73','#D55E00','#CC79A7','#56B4E9','#F0E442','#000000'],
      highContrast: ['#003F5C','#BC5090','#FFA600','#2F4B7C','#FF6361','#58508D'],
      monochrome: ['#0F172A','#334155','#475569','#64748B','#94A3B8','#CBD5E1'],
    })
    for (const palette of Object.keys(CHART_PALETTES) as ChartPalette[]) {
      const model = { ...models[1], presentation: presentation(palette) } as ChartModel
      expect(compileView(model).color).toEqual(CHART_PALETTES[palette])
      expect(compileView(model)).toEqual(compileView(model))
    }
  })

  it('maps controlled title, grid, legend and line presentation', () => {
    const option = compileView(models[0] as LineChartModel)
    expect(option.title).toMatchObject({ show: true, text: '标题' })
    expect(option.legend).toMatchObject({ show: true, right: 16, top: 'middle', orient: 'vertical' })
    expect((option.yAxis as { splitLine: { show: boolean } }).splitLine.show).toBe(true)
    expect((option.series as { smooth: boolean; showSymbol: boolean; lineStyle: unknown }[])[0]).toMatchObject({
      smooth: false, showSymbol: true, lineStyle: { width: 3, type: 'dashed' },
    })
  })

  it('downgrades a right vertical legend on a 390px viewport without hiding it', () => {
    const legend = compileView(models[0], { narrow: true }).legend
    expect(legend).toMatchObject({ show: true, left: 'center', bottom: 0, orient: 'horizontal' })
  })

  it('describes rendered series and the resolved responsive legend position', () => {
    const model={...models[0],series:['A相','B相','C相'].map((name,index)=>({name,points:[{x:{kind:'string' as const,value:'bus'},y:index+1,ordinal:0}]}))} as LineChartModel
    expect(chartAriaLabel(model,false)).toBe('line 图表，3 个系列：A相、B相、C相，图例右侧')
    expect(chartAriaLabel(model,true)).toBe('line 图表，3 个系列：A相、B相、C相，图例底部')
  })

  it('does not render a disabled legend, hidden title or grid', () => {
    const model = { ...models[1], title: null, presentation: {
      ...presentation(), legend: { show: false, position: 'auto' as const, orientation: 'auto' as const },
      title: { show: false }, axis: { showGrid: false },
    } } as ChartModel
    const option = compileView(model)
    expect(option.legend).toMatchObject({ show: false })
    expect(option.title).toBeUndefined()
    expect((option.yAxis as { splitLine: { show: boolean } }).splitLine.show).toBe(false)
  })
})
