import { useEffect, useRef, useState } from 'react'
import {
  deleteDataResource, downloadDataResource, previewDataResourceDeletion, uploadReplacementFiles,
} from '../../api/client'
import { useDataResources } from '../../hooks/useDataResources'
import type { DataResourceDeletionPreview, DataResourceView, UploadBatchActions, UploadBatchView } from '../../types'
import UploadBatchDecisionCard from '../uploads/UploadBatchDecisionCard'
import DataResourceDeleteDialog from './DataResourceDeleteDialog'
import DataResourceList from './DataResourceList'
import DataResourceWorkbench from './DataResourceWorkbench'

const operationId=()=>crypto.randomUUID?.()??`${Date.now()}-${Math.random()}`

export default function DataResourcePanel({sessionId,batches,actions,notify,onChanged,onModifyParameters}:{sessionId:string|null;batches:UploadBatchView[];actions?:UploadBatchActions;notify:(text:string)=>void;onChanged:()=>void;onModifyParameters?:(resource:DataResourceView)=>void}){
  const{resources,loading,error,nextCursor,reload,loadMore}=useDataResources(sessionId)
  const[preview,setPreview]=useState<DataResourceDeletionPreview|null>(null),[busyId,setBusyId]=useState<string|null>(null),[openId,setOpenId]=useState<string|null>(null)
  const terminalBatchKey=batches.filter(batch=>['confirmed','partially_confirmed'].includes(batch.status)).map(batch=>`${batch.id}:${batch.revision}:${batch.status}`).join('|'),previousTerminalKey=useRef(terminalBatchKey)
  useEffect(()=>{if(previousTerminalKey.current!==terminalBatchKey){previousTerminalKey.current=terminalBatchKey;void reload()}},[terminalBatchKey,reload])
  const saveDownload=async(resource:DataResourceView,kind:'normalized'|'original')=>{try{const{blob,filename}=await downloadDataResource(resource.id,kind,resource.alias);const url=URL.createObjectURL(blob),anchor=document.createElement('a');anchor.href=url;anchor.download=filename;anchor.click();URL.revokeObjectURL(url)}catch(cause){notify(cause instanceof Error?cause.message:'下载失败')}}
  const startDelete=async(resource:DataResourceView)=>{setBusyId(resource.id);try{setPreview(await previewDataResourceDeletion(resource.id))}catch(cause){notify(cause instanceof Error?cause.message:'删除影响读取失败')}finally{setBusyId(null)}}
  const confirmDelete=async()=>{if(!preview)return;setBusyId(preview.resourceId);try{await deleteDataResource(preview.resourceId,preview.impactToken);setPreview(null);await reload();onChanged();notify('数据文件已删除')}catch(cause){notify(cause instanceof Error?cause.message:'删除失败')}finally{setBusyId(null)}}
  const replace=async(resource:DataResourceView,file:File)=>{setBusyId(resource.id);try{const batch=await uploadReplacementFiles(resource.id,operationId(),[{id:operationId(),file}]);actions?.addBatch(batch);actions?.watch(batch.id);notify('替换文件已进入确认')}catch(cause){notify(cause instanceof Error?cause.message:'重新上传失败')}finally{setBusyId(null)}}
  const visibleBatches=batches.filter(batch=>!['confirmed','discarded','expired'].includes(batch.status))
  return <div className="space-y-3 p-3">
    {visibleBatches.map(batch=><UploadBatchDecisionCard key={`${batch.id}:${batch.revision}`} batch={batch} actions={actions} notify={notify}/>) }
    <div className="flex items-center"><h2 className="text-sm font-semibold">数据文件</h2><button type="button" disabled={loading} onClick={()=>void reload()} className="ml-auto rounded border bg-white px-2 py-1 text-xs">刷新</button></div>
    {error&&<p role="alert" className="text-xs text-red-600">{error}</p>}
    {!loading&&resources.length===0&&<p className="rounded-xl border bg-white p-4 text-xs text-slate-400">暂无数据文件。可从消息输入区上传 CSV、Excel 或 Parquet。</p>}
    <DataResourceList resources={resources} busyId={busyId} onView={resource=>setOpenId(resource.id)} onDownload={(resource,kind)=>void saveDownload(resource,kind)} onDelete={resource=>void startDelete(resource)} onReplace={(resource,file)=>void replace(resource,file)} onModifyParameters={resource=>onModifyParameters?onModifyParameters(resource):notify('请在研究配置卡中修改该参数资源')}/>
    {nextCursor&&<button type="button" disabled={loading} onClick={()=>void loadMore()} className="w-full rounded border bg-white py-2 text-xs">{loading?'加载中…':'加载更多'}</button>}
    {preview&&<DataResourceDeleteDialog preview={preview} busy={busyId===preview.resourceId} onCancel={()=>setPreview(null)} onConfirm={()=>void confirmDelete()}/>} {openId&&<DataResourceWorkbench resourceId={openId} onClose={()=>setOpenId(null)}/>} 
  </div>
}
