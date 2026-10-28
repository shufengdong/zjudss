import type { UploadBatchActions, UploadBatchMessage, UploadBatchView } from '../../types'
import UploadBatchDecisionCard from '../uploads/UploadBatchDecisionCard'

export default function UploadBatchCard({ message, batch, actions, onOpen, notify = () => undefined }: {
  message: UploadBatchMessage
  batch?: UploadBatchView
  actions?: UploadBatchActions
  onOpen?: () => void
  notify?: (text: string) => void
}) {
  if (batch) return <div className="mx-auto my-3 max-w-3xl"><UploadBatchDecisionCard batch={batch} actions={actions} notify={notify} /></div>
  return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm">
    <div className="flex items-center"><b>上传批次 #{message.card.batchId}</b><span className="ml-auto text-xs text-slate-500">{message.card.state}</span></div>
    <p className="mt-1 text-xs text-slate-500">模型：{message.card.modelTransactionState} · 数据：{message.card.resourceTransactionState}</p>
    {message.card.candidates.length > 0 && <ul className="mt-2 space-y-1 text-xs">{message.card.candidates.map((candidate) => <li key={candidate.id}>{candidate.name} · {candidate.state}</li>)}</ul>}
    {message.card.diagnostic && <p role="alert" className="mt-2 text-xs text-red-600">{message.card.diagnostic.message}</p>}
    {onOpen && <button type="button" onClick={onOpen} className="mt-3 rounded border px-3 py-1.5 text-xs">查看并确认</button>}
  </article>
}
