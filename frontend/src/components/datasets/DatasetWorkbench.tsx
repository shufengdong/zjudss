import{useState}from'react';import{downloadDataset}from'../../api/client';import{useDataset}from'../../hooks/useDataset';import{useDatasetView}from'../../hooks/useDatasetView'
import DatasetTable from'./DatasetTable';import ChartRenderer from'./ChartRenderer'
export default function DatasetWorkbench({datasetId,onClose}:{datasetId:string;onClose:()=>void}){
  const{metadata,page,loading,error,loadMore}=useDataset(datasetId),[viewId,setViewId]=useState<string|null>(null),view=useDatasetView(viewId)
  const download=async()=>{if(!metadata)return;const{blob,filename}=await downloadDataset(datasetId,metadata.name);const url=URL.createObjectURL(blob);const a=document.createElement('a');a.href=url;a.download=filename;a.click();URL.revokeObjectURL(url)}
  return <div className="flex h-full min-w-[560px] flex-col bg-white"><header className="flex h-12 items-center gap-2 border-b px-4">
    <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{metadata?.name??'数据集'}</h2>
    <button onClick={()=>void download()} className="rounded border px-2 py-1 text-xs">下载 Parquet</button><button onClick={onClose}>×</button>
  </header>{error?<div className="m-4 rounded bg-red-50 p-3 text-sm text-red-600">{error}</div>:metadata&&<>
    <nav className="flex gap-1 border-b px-3 py-2"><button onClick={()=>setViewId(null)} className={`rounded px-3 py-1 text-xs ${!viewId?'bg-brand-50 text-brand-700':''}`}>数据表</button>
      {metadata.views.map(v=><button key={v.id} onClick={()=>setViewId(v.id)} className={`rounded px-3 py-1 text-xs ${viewId===v.id?'bg-brand-50 text-brand-700':''}`}>{v.name}</button>)}</nav>
    <div className="flex-1 overflow-auto">{viewId?(view.error?<div className="m-4 rounded bg-amber-50 p-3 text-sm text-amber-700">{view.error}。可在对话中要求筛选或聚合。</div>:view.data&&<ChartRenderer data={view.data} schema={metadata.columns}/>):<DatasetTable metadata={metadata} page={page} loading={loading} onMore={loadMore}/>}</div>
  </>}</div>
}
