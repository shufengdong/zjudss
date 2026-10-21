import { useEffect, useRef } from 'react'
import type { KeyboardEvent } from 'react'
import type { Attachment } from '../types'
import { PaperclipIcon, SendIcon, XIcon } from './icons'

export interface PendingAttachment extends Attachment {
  id: string
}

interface ComposerProps {
  value: string
  onChange: (value: string) => void
  onSend: () => void
  attachments: PendingAttachment[]
  onAddAttachment: () => void
  onRemoveAttachment: (id: string) => void
}

export default function Composer({
  value,
  onChange,
  onSend,
  attachments,
  onAddAttachment,
  onRemoveAttachment,
}: ComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`
  }, [value])

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      onSend()
    }
  }

  const canSend = value.trim().length > 0 || attachments.length > 0

  return (
    <div className="border-t border-slate-200 bg-white px-4 py-3">
      {attachments.length > 0 && (
        <div className="mb-2 flex flex-wrap gap-2">
          {attachments.map((a) => (
            <span
              key={a.id}
              className="inline-flex items-center gap-1.5 rounded-full bg-slate-100 py-1 pl-2.5 pr-1.5 text-xs text-slate-600"
            >
              <span className="max-w-[160px] truncate">{a.fileName}</span>
              <button
                type="button"
                onClick={() => onRemoveAttachment(a.id)}
                className="rounded-full p-0.5 text-slate-400 hover:bg-slate-200 hover:text-slate-600"
                title="移除"
              >
                <XIcon className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      <div className="flex items-end gap-2 rounded-2xl border border-slate-200 bg-slate-50 px-3 py-2 transition-colors focus-within:border-brand-400">
        <button
          type="button"
          onClick={onAddAttachment}
          className="mb-0.5 shrink-0 text-slate-400 transition-colors hover:text-brand-600"
          title="上传文件（mock）"
        >
          <PaperclipIcon className="h-5 w-5" />
        </button>
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={handleKeyDown}
          rows={1}
          placeholder="输入仿真需求，Enter 发送，Shift+Enter 换行"
          className="max-h-40 flex-1 resize-none bg-transparent py-1 text-sm leading-relaxed text-slate-700 outline-none placeholder:text-slate-400"
        />
        <button
          type="button"
          onClick={onSend}
          disabled={!canSend}
          className="mb-0.5 shrink-0 rounded-lg bg-brand-500 p-2 text-white transition-colors hover:bg-brand-600 disabled:cursor-not-allowed disabled:opacity-40"
          title="发送"
        >
          <SendIcon className="h-4 w-4" />
        </button>
      </div>

      <p className="mt-1.5 px-1 text-[11px] text-slate-400">仿真内核尚未接入，当前为静态演示。</p>
    </div>
  )
}
