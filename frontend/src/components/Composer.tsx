import { useEffect, useRef, useState } from 'react'
import type { ChangeEvent, KeyboardEvent } from 'react'
import { PaperclipIcon, SendIcon } from './icons'

interface ComposerProps {
  value: string
  onChange: (value: string) => void
  onSend: () => void
  uploading: boolean
  onUploadFiles: (files: File[]) => void
  onCreateScript: (content: string) => Promise<void>
}

export default function Composer({
  value,
  onChange,
  onSend,
  uploading,
  onUploadFiles,
  onCreateScript,
}: ComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const directoryRef = useRef<HTMLInputElement>(null)
  const [scriptOpen, setScriptOpen] = useState(false)
  const [scriptContent, setScriptContent] = useState('')
  const [scriptSubmitting, setScriptSubmitting] = useState(false)
  const [scriptError, setScriptError] = useState<string | null>(null)

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

  const submitScript = async () => {
    if (!scriptContent.trim()) {
      setScriptError('脚本内容不能为空')
      return
    }
    setScriptSubmitting(true)
    setScriptError(null)
    try {
      await onCreateScript(scriptContent)
      setScriptOpen(false)
      setScriptContent('')
    } catch (error) {
      setScriptError(error instanceof Error ? error.message : '创建失败')
    } finally {
      setScriptSubmitting(false)
    }
  }

  const canSend = value.trim().length > 0

  return (
    <div className="border-t border-slate-200 bg-white px-4 py-3">
      <div className="flex items-end gap-2 rounded-2xl border border-slate-200 bg-slate-50 px-3 py-2 transition-colors focus-within:border-brand-400">
        <input ref={fileRef} type="file" multiple className="hidden" onChange={handleFiles} />
        <input ref={directoryRef} type="file" multiple className="hidden" onChange={handleFiles} {...{webkitdirectory:''}} />
        <button
          type="button"
          aria-label="上传文件"
          onClick={() => fileRef.current?.click()}
          disabled={uploading}
          className="mb-0.5 shrink-0 text-slate-400 transition-colors hover:text-brand-600 disabled:cursor-not-allowed disabled:opacity-40"
          title="上传文件"
        >
          <PaperclipIcon className="h-5 w-5" />
        </button>
        <button type="button" aria-label="上传文件夹" onClick={()=>directoryRef.current?.click()} disabled={uploading}
          className="mb-0.5 shrink-0 text-[11px] text-slate-400 hover:text-brand-600 disabled:opacity-40" title="上传文件夹">文件夹</button>
        <button type="button" aria-label="新建脚本" onClick={()=>setScriptOpen(true)} disabled={uploading}
          className="mb-0.5 shrink-0 text-[11px] text-slate-400 hover:text-brand-600 disabled:opacity-40" title="粘贴脚本文本">脚本</button>
        <textarea
          aria-label="消息"
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
          aria-label="发送消息"
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

      {scriptOpen && (
        <div
          className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-4"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setScriptOpen(false)
          }}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="script-dialog-title"
            className="w-full max-w-lg rounded-2xl bg-white p-6 shadow-2xl"
          >
            <h2 id="script-dialog-title" className="text-base font-semibold text-slate-900">新建脚本</h2>
            <textarea
              aria-label="脚本内容"
              value={scriptContent}
              onChange={(e) => setScriptContent(e.target.value)}
              rows={10}
              placeholder="粘贴 RSS 脚本内容"
              className="mt-2 w-full resize-y rounded-lg border border-slate-300 px-3 py-2 font-mono text-xs outline-none focus:border-brand-400"
            />
            {scriptError && <p role="alert" className="mt-2 text-xs text-red-600">{scriptError}</p>}
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setScriptOpen(false)}
                className="rounded-lg border border-slate-300 px-4 py-2 text-sm text-slate-700 hover:bg-slate-50"
              >
                取消
              </button>
              <button
                type="button"
                disabled={scriptSubmitting}
                onClick={() => void submitScript()}
                className="rounded-lg bg-brand-500 px-4 py-2 text-sm font-medium text-white hover:bg-brand-600 disabled:opacity-40"
              >
                {scriptSubmitting ? '创建中…' : '创建'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
