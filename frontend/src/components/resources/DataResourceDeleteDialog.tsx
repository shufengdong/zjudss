import type { DataResourceDeletionPreview } from '../../types'

export default function DataResourceDeleteDialog({preview,busy,onCancel,onConfirm}:{preview:DataResourceDeletionPreview;busy:boolean;onCancel:()=>void;onConfirm:()=>void}){
  return <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-4">
    <div role="dialog" aria-modal="true" aria-label="删除数据文件" className="w-full max-w-md rounded-xl bg-white p-5 shadow-2xl">
      <h2 className="font-semibold">删除数据文件</h2>
      <p className="mt-2 text-sm">将删除“{preview.displayName}”（{preview.alias}）及其当前文件。</p>
      <ul className="mt-3 space-y-1 text-xs text-slate-600">
        <li>{preview.pendingConfigurationCount} 个未确认配置将受影响</li>
        <li>{preview.frozenReferenceCount} 个冻结引用保留历史 artifact</li>
        <li>{preview.historicalReferenceCount} 个历史引用保留血缘</li>
      </ul>
      <div className="mt-5 flex justify-end gap-2"><button type="button" disabled={busy} onClick={onCancel} className="rounded border px-3 py-1.5 text-sm">取消</button><button type="button" disabled={busy} onClick={onConfirm} className="rounded bg-red-600 px-3 py-1.5 text-sm text-white">{busy?'删除中…':'确认删除'}</button></div>
    </div>
  </div>
}
