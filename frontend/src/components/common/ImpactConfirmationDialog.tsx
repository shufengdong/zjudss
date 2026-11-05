import { useEffect, useId, useRef, type ReactNode } from 'react'

export interface ImpactConfirmationDialogProps {
  title: string
  children?: ReactNode
  loading: boolean
  error: string | null
  busy: boolean
  confirmLabel?: string
  busyLabel?: string
  confirmDisabled?: boolean
  onCancel: () => void
  onConfirm: () => void
}

const focusable = 'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'

export default function ImpactConfirmationDialog({
  title,
  children,
  loading,
  error,
  busy,
  confirmLabel = '确认删除',
  busyLabel = '正在删除…',
  confirmDisabled = false,
  onCancel,
  onConfirm,
}: ImpactConfirmationDialogProps) {
  const titleId = useId()
  const descriptionId = useId()
  const dialog = useRef<HTMLDivElement | null>(null)
  const cancel = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    if (!busy) cancel.current?.focus()
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (!busy) { event.preventDefault(); onCancel() }
        return
      }
      if (event.key !== 'Tab') return
      const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>(focusable) ?? [])
      if (controls.length === 0) return
      const first = controls[0]
      const last = controls[controls.length - 1]
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus() }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus() }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [busy, onCancel])

  return <div
    data-testid="impact-dialog-backdrop"
    className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-4"
    onMouseDown={event => { if (!busy && event.target === event.currentTarget) onCancel() }}
  >
    <div
      ref={dialog}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      className="max-h-[calc(100dvh-2rem)] w-full max-w-md overflow-y-auto rounded-xl bg-white p-5 shadow-2xl"
    >
      <h2 id={titleId} className="font-semibold">{title}</h2>
      <div id={descriptionId} className="mt-2">
        {loading && <p className="text-sm text-slate-500">正在计算删除影响…</p>}
        {!loading && children}
        {error && <p role="alert" className="mt-3 text-sm text-red-600">{error}</p>}
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <button ref={cancel} type="button" disabled={busy} onClick={onCancel} className="rounded border px-3 py-1.5 text-sm">取消</button>
        <button type="button" disabled={busy || loading || confirmDisabled} onClick={onConfirm} className="rounded bg-red-600 px-3 py-1.5 text-sm text-white disabled:bg-slate-300">{busy ? busyLabel : confirmLabel}</button>
      </div>
    </div>
  </div>
}
