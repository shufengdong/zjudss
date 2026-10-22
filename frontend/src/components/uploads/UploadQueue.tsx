import type { UploadEntry } from '../../hooks/useIngestion'
export default function UploadQueue({entries}:{entries:UploadEntry[]}){
  if(!entries.length)return null
  return <div className="space-y-1 border-t border-slate-100 px-4 py-2">
    {entries.map(e=><div key={e.id} aria-label={`上传 ${e.relativePath??e.name}：${e.status}`} className="flex gap-2 text-xs">
      <span className="min-w-0 flex-1 truncate text-slate-600">{e.relativePath??e.name}</span>
      <span className={e.status==='failed'?'text-red-500':'text-slate-400'}>{e.status}</span>
      {e.error&&<span className="text-red-500">{e.error}</span>}
    </div>)}
  </div>
}
