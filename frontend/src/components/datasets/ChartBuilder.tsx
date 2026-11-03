import { useEffect, useMemo, useState } from 'react'
import { createChartView } from '../../api/client'
import { useChartCapabilities } from '../../hooks/useChartCapabilities'
import { chartConfigFingerprint, useChartPreview } from '../../hooks/useChartPreview'
import type { ChartConfig, ChartFieldRef, ChartKind, ChartPalette, ChartPresentation } from '../../types'
import ChartFieldSelect, { chartFieldOptions } from './ChartFieldSelect'
import ChartPreview from './ChartPreview'

const kinds: { value:ChartKind; label:string }[] = [
  {value:'line',label:'折线图'},{value:'bar',label:'柱状图'},{value:'scatter',label:'散点图'},
  {value:'heatmap',label:'热力图'},{value:'boxplot',label:'箱线图'},{value:'histogram',label:'直方图'},
]
const presentation = (title:string):ChartPresentation => ({ legend:{show:true,position:'auto',orientation:'auto'},
  palette:'colorblind',title:{show:Boolean(title)},axis:{showGrid:true} })
const ref = (key:string, options:ReturnType<typeof chartFieldOptions>):ChartFieldRef => options.find(option=>option.key===key)?.field ?? options.find(option=>!option.disabled)!.field

export default function ChartBuilder({datasetId,sourceName,onSaved,notify}:{datasetId:string;sourceName:string;onSaved:(viewId:string)=>void;notify:(text:string)=>void}) {
  const capabilities=useChartCapabilities(datasetId),preview=useChartPreview(datasetId)
  const [kind,setKind]=useState<ChartKind>('line'),[name,setName]=useState(`${sourceName} 图表`),[title,setTitle]=useState('')
  const [x,setX]=useState(''),[numericX,setNumericX]=useState(''),[y,setY]=useState<string[]>([]),[secondCategory,setSecondCategory]=useState('')
  const [xAxisMode,setXAxisMode]=useState<'category'|'value'>('category'),[sampling,setSampling]=useState(false),[samplePoints,setSamplePoints]=useState(1000)
  const [order,setOrder]=useState<'source'|'xAsc'|'xDesc'>('source'),[aggregation,setAggregation]=useState<'none'|'mean'|'min'|'max'|'sum'|'count'>('none')
  const [palette,setPalette]=useState<ChartPalette>('colorblind'),[legend,setLegend]=useState(true),[legendPosition,setLegendPosition]=useState<'auto'|'top'|'bottom'|'right'>('auto'),[legendOrientation,setLegendOrientation]=useState<'auto'|'horizontal'|'vertical'>('auto'),[grid,setGrid]=useState(true)
  const [lineWidth,setLineWidth]=useState<1|2|3|4>(2),[lineStyle,setLineStyle]=useState<'solid'|'dashed'|'dotted'>('solid'),[showSymbol,setShowSymbol]=useState(false)
  const [bins,setBins]=useState(20),[showOutliers,setShowOutliers]=useState(true),[saving,setSaving]=useState(false)
  const categories=useMemo(()=>capabilities.data?chartFieldOptions(capabilities.data,'category'):[],[capabilities.data])
  const measures=useMemo(()=>capabilities.data?chartFieldOptions(capabilities.data,'measure'):[],[capabilities.data])
  const measureLimit=Math.min(16,capabilities.data?.limits.maxSeries??16)
  useEffect(()=>{if(!capabilities.data)return;const category=categories.find(option=>!option.disabled)?.key??'', enabled=measures.filter(option=>!option.disabled), first=enabled[0]
    const unit=first?capabilities.data.fields.find(field=>field.column===first.field.column)?.unit:null
    const preferred=first?enabled.filter(option=>option.field.column===first.field.column||capabilities.data!.fields.find(field=>field.column===option.field.column)?.unit===unit):[]
    const numeric=preferred.slice(0,measureLimit).map(option=>option.key), enabledKind=capabilities.data.templates.find(template=>template.enabled)?.kind??'line'
    if(!capabilities.data.templates.find(template=>template.kind===kind)?.enabled)setKind(enabledKind)
    setX(category||first?.key||'');setNumericX(first?.key??'');setSecondCategory(category);setY(numeric);if(!category)setXAxisMode('value');preview.clear()
  },[capabilities.data,datasetId]) // preview.clear is stable; resetting is intentional per capability generation
  const config=useMemo<ChartConfig|null>(()=>{if(!capabilities.data||y.length===0)return null
    if(!capabilities.data.templates.find(template=>template.kind===kind)?.enabled)return null
    if(kind==='scatter'&&!numericX)return null
    if(kind!=='scatter'&&kind!=='histogram'&&!x)return null
    const base=presentation(title);base.palette=palette;base.legend={show:legend,position:legendPosition,orientation:legendOrientation};base.axis.showGrid=grid
    const category=()=>ref(x,categories), measure=(key=y[0])=>ref(key,measures)
    const scalarAggregation=aggregation==='count'?'mean':aggregation
    if(kind==='line')return{version:1,kind,title:title||undefined,presentation:{...base,line:{width:lineWidth,style:lineStyle,showSymbol}},x:xAxisMode==='value'?ref(x,measures):category(),xAxisMode,measures:y.map(key=>measure(key)),order,aggregation:scalarAggregation,sampling:sampling&&xAxisMode==='value'?{mode:'lttb',maxPointsPerSeries:samplePoints}:{mode:'none'}}
    if(kind==='bar')return{version:1,kind,title:title||undefined,presentation:base,x:category(),measures:y.map(key=>measure(key)),order,aggregation:scalarAggregation}
    if(kind==='scatter')return{version:1,kind,title:title||undefined,presentation:base,x:ref(numericX,measures),measures:y.map(key=>measure(key))}
    if(kind==='heatmap')return{version:1,kind,title:title||undefined,presentation:base,x:category(),y:ref(secondCategory||x,categories),value:measure(),aggregation:aggregation==='none'?'mean':aggregation}
    if(kind==='boxplot')return{version:1,kind,title:title||undefined,presentation:base,category:category(),value:measure(),showOutliers}
    return{version:1,kind:'histogram',title:title||undefined,presentation:base,value:measure(),bins}
  },[capabilities.data,kind,title,palette,legend,legendPosition,legendOrientation,grid,x,numericX,y,secondCategory,xAxisMode,sampling,samplePoints,order,aggregation,lineWidth,lineStyle,showSymbol,bins,showOutliers,categories,measures])
  const fingerprint=config?chartConfigFingerprint(config):null, fresh=Boolean(fingerprint&&preview.previewHash===fingerprint&&preview.model)
  const save=async()=>{if(!config||!fresh)return;setSaving(true);try{const response=await createChartView(datasetId,{clientOperationId:crypto.randomUUID(),name,config});notify('图表已保存');onSaved(response.view.id)}catch(value){notify(value instanceof Error?value.message:'图表保存失败')}finally{setSaving(false)}}
  if(capabilities.loading)return <p className="p-4 text-sm text-slate-500">正在读取可视化字段…</p>
  if(capabilities.error)return <div role="alert" className="m-4 text-sm text-red-600">{capabilities.error}<button onClick={capabilities.reload}>重试</button></div>
  if(!capabilities.data)return null
  const template=capabilities.data.templates.find(item=>item.kind===kind)
  const reasonLabel=(reason:string|null|undefined)=>reason==='no_category_field'?'缺少分类字段':reason==='no_numeric_field'?'缺少数值字段':reason==='capability_truncated'?'字段分量过多':'当前数据不兼容'
  return <div className="grid gap-4 p-4 lg:grid-cols-[minmax(280px,360px)_minmax(420px,1fr)]"><fieldset disabled={saving} className="contents"><section className="grid content-start gap-3">
    <label className="grid gap-1 text-sm">图表类型<select aria-label="图表类型" value={kind} onChange={event=>{const next=event.target.value as ChartKind;setKind(next);if(next==='heatmap'&&aggregation==='none')setAggregation('mean');if(next!=='heatmap'&&aggregation==='count')setAggregation('mean');if(next!=='histogram'&&next!=='scatter')setX(categories.find(option=>!option.disabled)?.key??'');preview.clear()}} className="rounded border p-2">{kinds.map(item=>{const status=capabilities.data!.templates.find(template=>template.kind===item.value);return<option key={item.value} value={item.value} disabled={!status?.enabled}>{item.label}{status?.enabled?'':'（不可用）'}</option>})}</select></label>
    <label className="grid gap-1 text-sm">图表名称<input aria-label="图表名称" value={name} onChange={event=>setName(event.target.value)} className="rounded border p-2"/></label>
    <label className="grid gap-1 text-sm">标题<input aria-label="标题" value={title} onChange={event=>setTitle(event.target.value)} className="rounded border p-2"/></label>
    {kind==='scatter'?<ChartFieldSelect label="X 数值" options={measures} value={numericX} onChange={value=>setNumericX(value as string)}/>:kind==='histogram'?null:<ChartFieldSelect label={kind==='boxplot'?'分类':'X 轴'} options={kind==='line'&&xAxisMode==='value'?measures:categories} value={x} onChange={value=>setX(value as string)}/>}
    {kind==='heatmap'&&<ChartFieldSelect label="Y 轴" options={categories} value={secondCategory} onChange={value=>setSecondCategory(value as string)}/>}
    <ChartFieldSelect label={kind==='line'||kind==='bar'||kind==='scatter'?'数值列（可多选）':'数值列'} options={measures} value={kind==='line'||kind==='bar'||kind==='scatter'?y:(y[0]??'')} multiple={kind==='line'||kind==='bar'||kind==='scatter'} onChange={value=>setY((Array.isArray(value)?value:[value]).slice(0,measureLimit))}/>
    {kind==='histogram'&&<label className="text-sm">分箱数<input aria-label="分箱数" type="number" min={1} max={200} value={bins} onChange={event=>setBins(Number(event.target.value))}/></label>}
    {kind==='boxplot'&&<label className="text-sm"><input type="checkbox" checked={showOutliers} onChange={event=>setShowOutliers(event.target.checked)}/> 显示离群点</label>}
    <details><summary className="cursor-pointer text-sm">高级设置</summary>{(kind==='line'||kind==='bar'||kind==='heatmap')&&<div className="grid gap-2 py-2">{kind==='line'&&<label>X 轴模式<select aria-label="X 轴模式" value={xAxisMode} onChange={event=>{const mode=event.target.value as typeof xAxisMode;setXAxisMode(mode);setX((mode==='value'?measures:categories).find(option=>!option.disabled)?.key??'')}}><option value="category">分类</option><option value="value">数值</option></select></label>}<label>聚合<select aria-label="聚合" value={aggregation} onChange={event=>setAggregation(event.target.value as typeof aggregation)}>{kind!=='heatmap'&&<option value="none">不聚合</option>}<option value="mean">平均</option><option value="min">最小</option><option value="max">最大</option><option value="sum">求和</option>{kind==='heatmap'&&<option value="count">计数</option>}</select></label>{kind!=='heatmap'&&<label>顺序<select aria-label="顺序" value={order} onChange={event=>setOrder(event.target.value as typeof order)}><option value="source">源顺序</option><option value="xAsc">X 升序</option><option value="xDesc">X 降序</option></select></label>}{kind==='line'&&xAxisMode==='value'&&<><label><input type="checkbox" checked={sampling} onChange={event=>setSampling(event.target.checked)}/> LTTB 采样</label>{sampling&&<label>每系列点数<input aria-label="每系列点数" type="number" min={100} max={10000} value={samplePoints} onChange={event=>setSamplePoints(Number(event.target.value))}/></label>}</>}</div>}</details>
    <details><summary className="cursor-pointer text-sm">显示设置</summary><div className="grid gap-2 py-2"><label>调色板<select aria-label="调色板" value={palette} onChange={event=>setPalette(event.target.value as ChartPalette)}><option value="colorblind">色觉友好</option><option value="default">默认</option><option value="highContrast">高对比</option><option value="monochrome">单色</option></select></label><label><input type="checkbox" checked={legend} onChange={event=>setLegend(event.target.checked)}/> 显示图例</label>{legend&&<><label>图例位置<select aria-label="图例位置" value={legendPosition} onChange={event=>setLegendPosition(event.target.value as typeof legendPosition)}><option value="auto">自动</option><option value="top">顶部</option><option value="bottom">底部</option><option value="right">右侧</option></select></label><label>图例方向<select aria-label="图例方向" value={legendOrientation} onChange={event=>setLegendOrientation(event.target.value as typeof legendOrientation)}><option value="auto">自动</option><option value="horizontal">横向</option><option value="vertical">纵向</option></select></label></>}<label><input type="checkbox" checked={grid} onChange={event=>setGrid(event.target.checked)}/> 显示网格</label>{kind==='line'&&<><label>线宽<input aria-label="线宽" type="number" min={1} max={4} value={lineWidth} onChange={event=>setLineWidth(Number(event.target.value) as 1|2|3|4)}/></label><label>线型<select aria-label="线型" value={lineStyle} onChange={event=>setLineStyle(event.target.value as typeof lineStyle)}><option value="solid">实线</option><option value="dashed">虚线</option><option value="dotted">点线</option></select></label><label><input type="checkbox" checked={showSymbol} onChange={event=>setShowSymbol(event.target.checked)}/> 显示数据点</label></>}</div></details>
    {!template?.enabled&&<p className="text-sm text-amber-600">{reasonLabel(template?.reason)}</p>}
    <div className="flex gap-2"><button className="rounded bg-brand-600 px-3 py-2 text-sm text-white disabled:opacity-50" disabled={!config||preview.loading} onClick={()=>config&&void preview.run(config)}>生成预览</button><button className="rounded border px-3 py-2 text-sm disabled:opacity-50" disabled={!fresh||saving||!name.trim()} onClick={()=>void save()}>保存图表</button></div>
  </section></fieldset><ChartPreview model={preview.model} loading={preview.loading} error={preview.error} stale={Boolean(preview.model&&!fresh)}/></div>
}
