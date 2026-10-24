import{useState}from'react'
import type { ToolCallMessage, ToolStatus } from '../../types'
import { CheckIcon, XIcon } from '../icons'

function StatusPill({ status }: { status: ToolStatus }) {
  const config = {
    running: { text: '运行中', cls: 'bg-brand-50 text-brand-600' },
    success: { text: '成功', cls: 'bg-emerald-50 text-emerald-600' },
    failed: { text: '失败', cls: 'bg-red-50 text-red-600' },
  }[status]
  return <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${config.cls}`}>{config.text}</span>
}

function StatusIcon({ status }: { status: ToolStatus }) {
  if (status === 'running') {
    return <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-brand-500 border-t-transparent" />
  }
  if (status === 'success') {
    return (
      <span className="flex h-4 w-4 items-center justify-center rounded-full bg-emerald-500 text-white">
        <CheckIcon className="h-2.5 w-2.5" strokeWidth={3} />
      </span>
    )
  }
  return (
    <span className="flex h-4 w-4 items-center justify-center rounded-full bg-red-500 text-white">
      <XIcon className="h-2.5 w-2.5" strokeWidth={3} />
    </span>
  )
}

export default function ToolCallCard({ message }: { message: ToolCallMessage }) {
  const[expanded,setExpanded]=useState(false)
  const args = Object.entries(message.args)
    .map(([k, v]) => `${k}: ${JSON.stringify(v)}`)
    .join('，')

  return (
    <div className="mx-4 my-2 max-w-[78%] overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm sm:mx-6">
      <div className="flex items-center gap-2 px-3 py-2.5">
        <StatusIcon status={message.status} />
        <span className="min-w-0 truncate font-mono text-xs font-semibold text-slate-700">{message.toolName}</span>
        <button type="button" aria-expanded={expanded} onClick={()=>setExpanded(value=>!value)} className="ml-auto shrink-0 text-xs text-brand-600">{expanded?'收起':'查看详情'}</button>
        <div className="shrink-0">
          <StatusPill status={message.status} />
        </div>
      </div>
      {expanded&&<div className="max-h-64 space-y-2 overflow-auto border-t border-slate-100 px-3 py-2 text-xs text-slate-500">
        <div className="whitespace-pre-wrap break-all font-mono">{args||'无参数'}</div>
        {message.summary&&<div className="whitespace-pre-wrap break-words text-slate-400">{message.summary}</div>}
      </div>}
    </div>
  )
}
