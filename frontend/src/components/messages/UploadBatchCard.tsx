import type { UploadBatchMessage, UploadBatchView } from '../../types'
import { isClosedUploadBatch, uploadBatchWorkbenchTab } from '../uploads/batchPlacement'

const diagnosticMessages = (value: unknown): string[] => {
  const items = Array.isArray(value)
    ? value
    : value && typeof value === 'object' && Array.isArray((value as { errors?: unknown }).errors)
      ? (value as { errors: unknown[] }).errors
      : []
  return items.flatMap((item) => item && typeof item === 'object' && typeof (item as { message?: unknown }).message === 'string'
    ? [(item as { message: string }).message]
    : [])
}

export default function UploadBatchCard({ message, batch, onOpen }: {
  message: UploadBatchMessage
  batch?: UploadBatchView
  onOpen?: (batchId: string) => void
}) {
  const state=batch?.status??message.card.state
  const modelState=batch?.modelPublishStatus??message.card.modelTransactionState
  const resourceState=batch?.resourcePublishStatus??message.card.resourceTransactionState
  const candidateCount=batch?.candidates.length??message.card.candidates.length
  const diagnostics=batch?diagnosticMessages(batch.diagnostics):message.card.diagnostic?[message.card.diagnostic.message]:[]
  const actionLabel=!batch?'查看并确认':uploadBatchWorkbenchTab(batch)==='workspace'?'前往模型工作区':'前往数据文件'
  const actionable=Boolean(onOpen)&&(!batch||!isClosedUploadBatch(batch))
  return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm">
    <div className="flex items-center"><b>上传批次 #{message.card.batchId}</b><span className="ml-auto text-xs text-slate-500">{state}</span></div>
    <p className="mt-1 text-xs text-slate-500">{candidateCount} 个候选 · 模型：{modelState} · 数据：{resourceState}</p>
    {diagnostics.length>0&&<div role="alert" className="mt-2 text-xs text-red-600">{diagnostics.map(value=><p key={value}>{value}</p>)}</div>}
    {actionable&&<button type="button" onClick={()=>onOpen?.(message.card.batchId)} className="mt-3 rounded border px-3 py-1.5 text-xs">{actionLabel}</button>}
  </article>
}
