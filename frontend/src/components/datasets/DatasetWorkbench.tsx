import{useEffect,useState}from'react';import{downloadDataset}from'../../api/client';import{useDataset}from'../../hooks/useDataset';import{useDatasetView}from'../../hooks/useDatasetView'
import DatasetTable from'./DatasetTable';import ChartRenderer from'./ChartRenderer'
export default function DatasetWorkbench({datasetId,onClose,onDeleted}:{datasetId:string;onClose:()=>void;onDeleted?:()=>void}){
  const[selectedColumns,setSelectedColumns]=useState<string[]|null>(null)
  const{metadata,page,loading,error,loadMore,remove}=useDataset(datasetId,selectedColumns??undefined),[viewId,setViewId]=useState<string|null>(null),view=useDatasetView(viewId)
  useEffect(()=>{setSelectedColumns(null);setViewId(null)},[datasetId])
  const toggleColumn=(name:string)=>{if(!metadata)return;const all=metadata.columns.map(column=>column.name),current=selectedColumns??all
    const next=current.includes(name)?current.filter(column=>column!==name):all.filter(column=>column===name||current.includes(column))
    if(next.length>0)setSelectedColumns(next.length===all.length?null:next)
  }
  const download=async()=>{if(!metadata)return;const{blob,filename}=await downloadDataset(datasetId,metadata.name);const url=URL.createObjectURL(blob);const a=document.createElement('a');a.href=url;a.download=filename;a.click();URL.revokeObjectURL(url)}
  const discard=async()=>{if(!window.confirm('删除此结果？已经物化的分析、比较和图表仍会保留。'))return;await remove();onDeleted?.();onClose()}
  return <div className="flex h-full w-full min-w-0 flex-col bg-white sm:min-w-[560px]"><header className="flex h-12 items-center gap-2 border-b px-4">
    <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{metadata?.name??'数据集'}</h2>{metadata&&<details className="relative text-xs"><summary className="cursor-pointer rounded border px-2 py-1">选择列</summary><div className="absolute right-0 z-10 mt-1 min-w-36 rounded border bg-white p-2 shadow">
      {metadata.columns.map(column=>{const checked=selectedColumns===null||selectedColumns.includes(column.name);return <label key={column.name} className="flex gap-2 whitespace-nowrap py-1"><input aria-label={`显示列 ${column.name}`} type="checkbox" checked={checked} disabled={checked&&(selectedColumns?.length??metadata.columns.length)===1} onChange={()=>toggleColumn(column.name)}/>{column.name}</label>})}
    </div></details>}
    <button onClick={()=>void download()} className="rounded border px-2 py-1 text-xs">下载 Parquet</button><button onClick={()=>void discard()} className="rounded border border-red-200 px-2 py-1 text-xs text-red-600">删除结果</button><button aria-label="关闭数据集" onClick={onClose}>×</button>
  </header>{error&&<div role="alert" className="m-4 rounded bg-red-50 p-3 text-sm text-red-600">{error}。已加载数据仍可查看，可重试加载更多。</div>}{metadata&&<>
    <nav className="flex gap-1 border-b px-3 py-2"><button onClick={()=>setViewId(null)} className={`rounded px-3 py-1 text-xs ${!viewId?'bg-brand-50 text-brand-700':''}`}>数据表</button>
      {metadata.views.map(v=><button key={v.id} onClick={()=>setViewId(v.id)} className={`rounded px-3 py-1 text-xs ${viewId===v.id?'bg-brand-50 text-brand-700':''}`}>{v.name}</button>)}</nav>
    <div className="flex-1 overflow-auto">{viewId?(view.error?<div className="m-4 rounded bg-amber-50 p-3 text-sm text-amber-700">{view.error}。可在对话中要求筛选或聚合。</div>:view.data&&<ChartRenderer data={view.data} schema={metadata.columns}/>):<DatasetTable metadata={metadata} page={page} loading={loading} onMore={loadMore}/>}</div>
  </>}</div>
}
