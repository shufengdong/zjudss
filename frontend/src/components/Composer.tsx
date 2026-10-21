import { useEffect, useRef } from 'react'
import type { ChangeEvent, KeyboardEvent } from 'react'
import { PaperclipIcon, SendIcon } from './icons'

interface ComposerProps {
  value: string
  onChange: (value: string) => void
  onSend: () => void
  uploading: boolean
  onUploadFiles: (files: File[]) => void
}

export default function Composer({
  value,
  onChange,
  onSend,
  uploading,
  onUploadFiles,
}: ComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)

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

  const handleFiles = (e: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? [])
    if (files.length > 0) onUploadFiles(files)
    // 重置 input，允许连续选择同一文件。
    e.target.value = ''
  }

  const canSend = value.trim().length > 0

  return (
    <div className="border-t border-slate-200 bg-white px-4 py-3">
      <div className="flex items-end gap-2 rounded-2xl border border-slate-200 bg-slate-50 px-3 py-2 transition-colors focus-within:border-brand-400">
        <input ref={fileRef} type="file" multiple className="hidden" onChange={handleFiles} />
        <button
          type="button"
          onClick={() => fileRef.current?.click()}
          disabled={uploading}
          className="mb-0.5 shrink-0 text-slate-400 transition-colors hover:text-brand-600 disabled:cursor-not-allowed disabled:opacity-40"
          title="上传文件"
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

      {uploading ? (
        <p className="mt-1.5 px-1 text-[11px] text-brand-600">附件上传中…</p>
      ) : (
        <p className="mt-1.5 px-1 text-[11px] text-slate-400">上传模型 / 数据文件后会自动落盘并回显到对话。</p>
      )}
    </div>
  )
}
