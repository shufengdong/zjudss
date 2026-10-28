import { useEffect, useRef, useState } from 'react'
import { useManagedWorkflow } from '../../hooks/useManagedWorkflow'
import type { WorkflowProgressMessage } from '../../types'

const terminal = new Set(['succeeded', 'failed', 'cancelled_by_user', 'service_restarted'])

function failureText(value: unknown): string {
  if (value && typeof value === 'object') {
    const error = value as { code?: unknown; message?: unknown }
    if (typeof error.message === 'string') return error.code ? `${error.message}（${String(error.code)}）` : error.message
  }
  return typeof value === 'string' ? value : '研究执行失败'
}

export default function WorkflowProgressCard({ message, onRetry, onChanged, notify }: {
  message: WorkflowProgressMessage
  onRetry?: (scriptId: string) => Promise<void>
  onChanged?: () => void
  notify: (text: string) => void
}) {
  const { workflow, loading, error, cancel } = useManagedWorkflow(message.card.workflowId, message.revision ?? 0)
  const [now, setNow] = useState(Date.now())
  const [cancelling, setCancelling] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const [attemptsExpanded, setAttemptsExpanded] = useState(false)
  const terminalNotified = useRef(false)
  const state = workflow?.status ?? message.card.state
  useEffect(() => {
    if (terminal.has(state)) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [state])
  useEffect(() => {
    if (!terminal.has(state)) {
      terminalNotified.current = false
      return
    }
    if (workflow && !terminalNotified.current) {
      terminalNotified.current = true
      onChanged?.()
    }
  }, [onChanged, state, workflow])
  const attempts = workflow?.attempts ?? []
  const workflowElapsed = workflow
    ? Math.max(0, (terminal.has(state) && workflow.finishedAt ? new Date(workflow.finishedAt).getTime() : now) - new Date(workflow.createdAt).getTime())
    : 0
  const elapsed = Math.max(message.card.elapsedMs, workflowElapsed)
  const scriptId = workflow?.finalScriptResourceId ? String(workflow.finalScriptResourceId) : message.card.scriptId
  const outputDatasetIds = workflow?.outputDatasetIds ?? message.card.outputDatasetIds ?? []
  const stop = async () => {
    if (cancelling) return
    setCancelling(true)
    try {
      await cancel()
      notify('已取消研究工作流')
      onChanged?.()
    } catch (cause) {
      notify(cause instanceof Error ? cause.message : '取消失败')
    } finally {
      setCancelling(false)
    }
  }
  const retry = async () => {
    if (!scriptId || !onRetry || retrying) return
    setRetrying(true)
    try { await onRetry(scriptId) } finally { setRetrying(false) }
  }

  return <article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm" aria-label="研究工作流进度">
    <div className="flex items-center"><b>研究工作流 #{message.card.workflowId}</b><span className="ml-auto text-xs">{state}</span></div>
    <p className="mt-2 text-xs text-slate-600">执行 {message.card.executionIndex}/{message.card.executionMax} · 修正 {message.card.repairIndex}/{message.card.repairMax} · {(elapsed / 1000).toFixed(1)} 秒</p>
    {loading && <p className="mt-2 text-xs text-slate-400">读取详细进度…</p>}
    {error && <p role="alert" className="mt-2 text-xs text-red-600">{error}</p>}
    {attempts.length > 0 && <section className="mt-2 text-xs">
      <button type="button" aria-expanded={attemptsExpanded} onClick={() => setAttemptsExpanded((value) => !value)} className="rounded border px-2 py-1">
        执行尝试（{attempts.length}）
      </button>
      {attemptsExpanded && <div className="mt-2 space-y-2">
        {attempts.map((attempt) => <section key={attempt.id} className="rounded bg-slate-50 p-2">
          <p>第 {attempt.attemptNumber} 次 · {attempt.phase} · {attempt.status}{attempt.exitCode !== null && attempt.exitCode !== undefined ? ` · 退出码 ${attempt.exitCode}` : ''}</p>
          {attempt.scriptContentHash && <p className="mt-1 break-all text-slate-500">脚本 hash：{attempt.scriptContentHash}</p>}
          {attempt.sanitizedStderr && <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-white p-2">{attempt.sanitizedStderr}</pre>}
        </section>)}
      </div>}
    </section>}
    {state === 'succeeded' && <div className="mt-3 text-xs">
      <p className="text-emerald-700">脚本已生成并执行成功。</p>
      {outputDatasetIds.length > 0 && <ul className="mt-2">{outputDatasetIds.map((id) => <li key={id}>结果数据集 #{id}</li>)}</ul>}
    </div>}
    {['failed', 'service_restarted', 'cancelled_by_user'].includes(state) && <div role="alert" className="mt-3 text-xs text-red-700">
      <p>{failureText(workflow?.error ?? message.card.failureCode)}</p>
      {scriptId && onRetry && <button type="button" disabled={retrying} onClick={() => void retry()} className="mt-2 rounded border border-red-200 bg-white px-3 py-1.5">{retrying ? '提交中…' : '重新尝试'}</button>}
    </div>}
    {!terminal.has(state) && <button type="button" disabled={cancelling} onClick={() => void stop()} className="mt-3 rounded border px-3 py-1.5 text-xs">{cancelling ? '取消中…' : '取消'}</button>}
  </article>
}
