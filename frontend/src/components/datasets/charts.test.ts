import{describe,expect,it}from'vitest';import type{DatasetViewData,ResultColumn,VisualizationKind}from'../../types';import{compileView}from'./ChartRenderer'
const schema:ResultColumn[]=[{name:'time',dtype:'Utf8',nullable:false},{name:'group',dtype:'Utf8',nullable:false},{name:'value',dtype:'Float64',nullable:false,unit:'p.u.'},{name:'other',dtype:'Float64',nullable:false}]
const rows=[['t1','a',1,3],['t2','a',2,4],['t1','b',3,5]]
const specs:Record<VisualizationKind,Record<string,unknown>>={
 line:{kind:'line',x:'time',y:['value']},bar:{kind:'bar',x:'time',y:['value']},scatter:{kind:'scatter',x:'value',y:['other']},
 heatmap:{kind:'heatmap',x:'time',category:'group',value:'value'},boxplot:{kind:'boxplot',category:'group',value:'value'},
 histogram:{kind:'histogram',value:'value',bins:2},
}
const unsafeKey=(value:unknown):boolean=>!!value&&typeof value==='object'&&Object.entries(value).some(([k,v])=>k.toLowerCase().includes('formatter')||unsafeKey(v))
describe('controlled ECharts builders',()=>{
  for(const kind of Object.keys(specs)as VisualizationKind[])it(`builds ${kind} without executable formatter`,()=>{
    const data:DatasetViewData={viewId:'v',datasetId:'d',kind,spec:specs[kind],columns:schema.map(c=>c.name),rows,diagnostics:{}}
    const option=compileView(data,schema);expect(option.series).toBeTruthy();expect(unsafeKey(option)).toBe(false)
  })
  it('retains histogram bin edges',()=>{
    const data:DatasetViewData={viewId:'v',datasetId:'d',kind:'histogram',spec:specs.histogram,columns:schema.map(c=>c.name),rows,diagnostics:{}}
    const series=(compileView(data,schema).series as unknown as {custom:{binEdges:number[][]}}[])[0]
    expect(series.custom.binEdges).toEqual([[1,2],[2,3]])
  })
})
