import { deleteChartView } from '../../api/client'
import { useSavedCharts } from '../../hooks/useSavedCharts'

export default function SavedChartPanel({sessionId,revision,onOpen,notify}:{sessionId:string|null;revision:number;onOpen:(viewId:string)=>void;notify:(text:string)=>void}){
  const charts=useSavedCharts(sessionId,revision)
  const remove=async(id:string,name:string)=>{if(!window.confirm(`删除图表“${name}”？`))return;try{await deleteChartView(id);notify('图表已删除');charts.reload()}catch(value){notify(value instanceof Error?value.message:'图表删除失败')}}
  if(!sessionId)return <p className="p-4 text-sm text-slate-500">请选择会话</p>
  return <section className="grid gap-3 p-3" aria-label="已保存图表">
    {charts.error&&<div role="alert" className="rounded bg-red-50 p-3 text-sm text-red-600">{charts.error}<button onClick={charts.reload} className="ml-2 underline">重试</button></div>}
    {!charts.loading&&!charts.error&&charts.views.length===0&&<p className="py-8 text-center text-sm text-slate-500">暂无已保存图表</p>}
    {charts.views.map(view=><article key={view.id} className="rounded-lg border bg-white p-3"><div className="flex items-start gap-2"><div className="min-w-0 flex-1"><h3 className="truncate text-sm font-medium">{view.name}</h3><p className="truncate text-xs text-slate-500">{view.sourceName} · {view.kind} · {view.pointCount} 点</p>{view.sourceDeleted&&<span className="mt-1 inline-block rounded bg-amber-50 px-1.5 py-0.5 text-xs text-amber-700">来源已删除</span>}</div><button aria-label={`打开 ${view.name}`} onClick={()=>onOpen(view.id)} className="rounded border px-2 py-1 text-xs">打开</button><button aria-label={`删除 ${view.name}`} onClick={()=>void remove(view.id,view.name)} className="rounded border border-red-200 px-2 py-1 text-xs text-red-600">删除</button></div></article>)}
    {charts.loading&&<p className="text-center text-sm text-slate-500">加载中…</p>}
    {charts.nextCursor&&<button onClick={()=>void charts.loadMore()} disabled={charts.loading} className="rounded border bg-white px-3 py-2 text-sm">加载更多</button>}
  </section>
}
