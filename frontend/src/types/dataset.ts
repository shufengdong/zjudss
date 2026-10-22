export const VISUALIZATION_KINDS = ['line','bar','scatter','heatmap','boxplot','histogram'] as const
export type VisualizationKind = (typeof VISUALIZATION_KINDS)[number]
export interface ResultColumn { name:string; dtype:string; nullable:boolean; unit?:string|null }
export interface DatasetView {
  id:string; name:string; kind:VisualizationKind; spec:Record<string,unknown>; isDefault:boolean
}
export interface DatasetMetadata {
  id:string; taskId:string|null; analysisRunId:string|null; name:string; rowCount:number
  sizeBytes:number; columns:ResultColumn[]; views:DatasetView[]
}
export interface DatasetPage {
  datasetId:string; columns:string[]; rows:unknown[][]; nextCursor:string|null; diagnostics:Record<string,number>
}
export interface DatasetViewData {
  viewId:string; datasetId:string; kind:VisualizationKind; spec:Record<string,unknown>
  columns:string[]; rows:unknown[][]; diagnostics:Record<string,number>
}
export interface ResultOutputRef {
  datasetId:string; name:string; rowCount:number; columns:ResultColumn[]; previewRows?:unknown[][]
  previewTruncated:boolean; defaultViewId:string|null
  views:{id:string;name:string;kind:VisualizationKind;isDefault:boolean}[]
}
