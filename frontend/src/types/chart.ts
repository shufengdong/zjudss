export type ChartKind = 'line' | 'bar' | 'scatter' | 'heatmap' | 'boxplot' | 'histogram'
export type ChartPalette = 'default' | 'colorblind' | 'highContrast' | 'monochrome'
export type LegendPosition = 'auto' | 'top' | 'bottom' | 'right'
export type LegendOrientation = 'auto' | 'horizontal' | 'vertical'
export type ChartOrder = 'source' | 'xAsc' | 'xDesc'
export type ChartAggregation = 'none' | 'mean' | 'min' | 'max' | 'sum'

export interface ChartFieldRef { column: string; path?: number[]; label?: string }
export interface LegendPresentation { show: boolean; position: LegendPosition; orientation: LegendOrientation }
export interface ChartPresentation {
  legend: LegendPresentation
  palette: ChartPalette
  title: { show: boolean }
  axis: { showGrid: boolean }
}
export interface LineChartPresentation extends ChartPresentation {
  line: { width: 1 | 2 | 3 | 4; style: 'solid' | 'dashed' | 'dotted'; showSymbol: boolean }
}
export type ChartSampling = { mode: 'none' } | { mode: 'lttb'; maxPointsPerSeries: number }

export interface ChartConfigBase { version: 1; title?: string }
export interface LineChartConfig extends ChartConfigBase {
  kind: 'line'; presentation: LineChartPresentation; x: ChartFieldRef
  xAxisMode: 'category' | 'value'; measures: ChartFieldRef[]; splitBy?: ChartFieldRef
  order: ChartOrder; aggregation: ChartAggregation; sampling: ChartSampling
}
export interface BarChartConfig extends ChartConfigBase {
  kind: 'bar'; presentation: ChartPresentation; x: ChartFieldRef; measures: ChartFieldRef[]
  splitBy?: ChartFieldRef; order: ChartOrder; aggregation: ChartAggregation
}
export interface ScatterChartConfig extends ChartConfigBase {
  kind: 'scatter'; presentation: ChartPresentation; x: ChartFieldRef
  measures: ChartFieldRef[]; splitBy?: ChartFieldRef
}
export interface HeatmapChartConfig extends ChartConfigBase {
  kind: 'heatmap'; presentation: ChartPresentation; x: ChartFieldRef; y: ChartFieldRef
  value: ChartFieldRef; aggregation: 'mean' | 'min' | 'max' | 'sum' | 'count'
}
export interface BoxplotChartConfig extends ChartConfigBase {
  kind: 'boxplot'; presentation: ChartPresentation; category: ChartFieldRef; value: ChartFieldRef
  splitBy?: ChartFieldRef; showOutliers: boolean
}
export interface HistogramChartConfig extends ChartConfigBase {
  kind: 'histogram'; presentation: ChartPresentation; value: ChartFieldRef; bins: number
}
export type ChartConfig = LineChartConfig | BarChartConfig | ScatterChartConfig |
  HeatmapChartConfig | BoxplotChartConfig | HistogramChartConfig
export interface ChartPreviewRequest { config: ChartConfig }
export interface CreateChartViewRequest { clientOperationId: string; name: string; config: ChartConfig }

export type CategoryValue =
  | { kind: 'string'; value: string }
  | { kind: 'boolean'; value: boolean }
  | { kind: 'integer'; value: string }
  | { kind: 'number'; value: number }

export interface ChartDiagnostics {
  inputRows: number; candidatePoints: number; outputPoints: number; droppedMissing: number
  droppedNonFinite: number; missingComponents: number; aggregatedGroups: number; sampledPoints: number
}
export interface ChartModelBase {
  contractVersion: 'chart-model-v1'; viewId: string | null; sourceDatasetId: string
  sourceDeleted: boolean; name: string | null; title: string | null; configHash: string
  diagnostics: ChartDiagnostics
}
export interface LineChartModel extends ChartModelBase {
  kind: 'line'; presentation: LineChartPresentation; order: ChartOrder; aggregation: ChartAggregation
  xAxis: { type: 'category' | 'value'; name: string; unit: string | null }
  yAxis: { name: string | null; unit: string | null }
  series: { name: string; points: { x: CategoryValue | number; y: number; ordinal: number }[] }[]
}
export interface BarChartModel extends ChartModelBase {
  kind: 'bar'; presentation: ChartPresentation
  xAxis: { type: 'category'; name: string; unit: string | null }
  yAxis: { name: string | null; unit: string | null }; categories: CategoryValue[]
  series: { name: string; values: (number | null)[] }[]
}
export interface ScatterChartModel extends ChartModelBase {
  kind: 'scatter'; presentation: ChartPresentation
  xAxis: { type: 'value'; name: string; unit: string | null }
  yAxis: { name: string | null; unit: string | null }
  series: { name: string; points: { x: number; y: number; ordinal: number }[] }[]
}
export interface HeatmapChartModel extends ChartModelBase {
  kind: 'heatmap'; presentation: ChartPresentation
  xAxis: { name: string; categories: CategoryValue[] }
  yAxis: { name: string; categories: CategoryValue[] }
  xCategories: CategoryValue[]; yCategories: CategoryValue[]
  value: { name: string; unit: string | null; min: number; max: number }
  cells: { xIndex: number; yIndex: number; value: number }[]
}
export interface BoxplotChartModel extends ChartModelBase {
  kind: 'boxplot'; presentation: ChartPresentation
  categoryAxis: { name: string; categories: CategoryValue[] }
  valueAxis: { name: string; unit: string | null }
  series: { name: string; values: ([number, number, number, number, number] | null)[]
    outliers: { categoryIndex: number; value: number }[] }[]
}
export interface HistogramChartModel extends ChartModelBase {
  kind: 'histogram'; presentation: ChartPresentation
  value: { name: string; unit: string | null }
  bins: { start: number; end: number; count: number; includesEnd: boolean }[]
}
export type ChartModel = LineChartModel | BarChartModel | ScatterChartModel |
  HeatmapChartModel | BoxplotChartModel | HistogramChartModel

export type ChartScalarType = 'string' | 'boolean' | 'integer' | 'number' | 'unsupported'
export type ChartCapabilityReason = 'no_category_field' | 'no_numeric_field' | 'capability_truncated'
export type ChartFieldShape = { kind: 'scalar' } | {
  kind: 'list'; rank: number; observedPaths: number[][]; complete: boolean; truncated: boolean
  reason: 'too_many_components' | null
}
export interface ChartCapabilityField {
  column: string; dtype: string; nullable: boolean; unit: string | null; scalarType: ChartScalarType
  roles: ('category' | 'measure')[]; shape: ChartFieldShape
}
export interface ChartCapabilities {
  datasetId: string; schemaHash: string; rowCount: number; fields: ChartCapabilityField[]
  templates: { kind: ChartKind; enabled: boolean; reason: ChartCapabilityReason | null }[]
  limits: { maxFields: number; maxObservedPathsPerField: number; maxSeries: number; maxPoints: number
    maxCategories: number; maxHeatmapCells: number }
}

export interface ChartViewSummary {
  id: string; sessionId: string; sourceDatasetId: string; sourceName: string; sourceDeleted: boolean
  name: string; kind: ChartKind; pointCount: number; sizeBytes: number; createdAt: string
}
export interface ChartDataFrameColumn { name: string; dtype: string; nullable: boolean; unit: string | null }
export interface ChartViewMetadata extends ChartViewSummary {
  contractVersion: 'chart-model-v1'; config: ChartConfig; configHash: string
  sourceSchema: ChartDataFrameColumn[]; sourceSchemaHash: string; sourceContentHash: string
  plotSchema: ChartDataFrameColumn[]; plotSchemaHash: string; plotContentHash: string
  diagnostics: ChartDiagnostics
}
export interface CreateChartViewResponse { view: ChartViewMetadata; replayed: boolean }
export interface ChartViewPage { views: ChartViewSummary[]; nextCursor: string | null }
export interface DeleteChartViewResponse { viewId: string; tombstone: unknown }
