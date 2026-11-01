import { useEffect, useRef, useState } from 'react'
import WorkflowResultsCard from './WorkflowResultsCard'
import { useManagedWorkflow } from '../../hooks/useManagedWorkflow'
import type { WorkflowProgressMessage } from '../../types'
import type { WorkflowEngineLog } from '../../types/workflow'

const terminal = new Set(['succeeded', 'failed', 'cancelled_by_user', 'service_restarted'])

function failureText(value: unknown): string {
  if (value && typeof value === 'object') {
    const error = value as { code?: unknown; message?: unknown }
    if (typeof error.message === 'string') return error.code ? `${error.message}（${String(error.code)}）` : error.message
  }
  return typeof value === 'string' ? value : '研究执行失败'
}

function downloadLog(workflowId: string, log: WorkflowEngineLog) {
  const blob = new Blob([log.text], { type: 'text/plain;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `workflow-${workflowId}-${log.stream}-sanitized.log`
  anchor.click()
  URL.revokeObjectURL(url)
}

export default function WorkflowProgressCard({ message, onRetry, onChanged, notify, onOpenDataset }: {
  message: WorkflowProgressMessage
  onRetry?: (scriptId: string) => Promise<void>
  onChanged?: () => void
  notify: (text: string) => void
  onOpenDataset?: (datasetId: string) => void
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
  const attemptsNewestFirst = [...attempts].reverse()
  const diagnosisAttempt = attemptsNewestFirst.find((attempt) => (attempt.engineLogs?.length ?? 0) > 0)
    ?? attemptsNewestFirst.find((attempt) => Boolean(attempt.sanitizedStderr))
  const diagnosisLogs = diagnosisAttempt?.engineLogs?.length
    ? diagnosisAttempt.engineLogs
    : diagnosisAttempt?.sanitizedStderr
      ? [{ stream: 'stderr', text: diagnosisAttempt.sanitizedStderr, totalBytes: diagnosisAttempt.sanitizedStderr.length, sentBytes: diagnosisAttempt.sanitizedStderr.length, truncated: false }]
      : []
  const analysis = diagnosisAttempt?.failureAnalysis
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

  return <><article className="mx-auto my-3 max-w-3xl rounded-xl border bg-white p-3 text-sm" aria-label="研究工作流进度">
    <div className="flex items-center"><b>研究工作流 #{message.card.workflowId}</b><span className="ml-auto text-xs">{state}</span></div>
    <p className="mt-2 text-xs text-slate-600">执行 {message.card.executionIndex}/{message.card.executionMax} · 修正 {message.card.repairIndex}/{message.card.repairMax} · {(elapsed / 1000).toFixed(1)} 秒</p>
    {loading && !workflow && <p className="mt-2 text-xs text-slate-400">读取详细进度…</p>}
    {error && <p role="alert" className="mt-2 text-xs text-red-600">{error}</p>}
    {['failed', 'service_restarted'].includes(state) && <section className="mt-3 rounded-lg border border-red-200 bg-red-50 p-3 text-xs" aria-label="失败诊断">
      <h3 className="font-semibold text-red-800">失败诊断</h3>
      <p className="mt-1 text-red-700">{failureText(workflow?.error ?? message.card.failureCode)}</p>
      {diagnosisAttempt && <p className="mt-2 text-slate-600">
        第 {diagnosisAttempt.attemptNumber} 次执行
        {diagnosisAttempt.exitCode !== null && diagnosisAttempt.exitCode !== undefined ? ` · 退出码 ${diagnosisAttempt.exitCode}` : ''}
        {diagnosisAttempt.durationMs !== null && diagnosisAttempt.durationMs !== undefined ? ` · ${(diagnosisAttempt.durationMs / 1000).toFixed(1)} 秒` : ''}
      </p>}
      <div className="mt-3 space-y-3">
        {diagnosisLogs.map((log) => <section key={log.stream}>
          <div className="flex items-center gap-2">
            <h4 className="font-medium text-slate-800">引擎 {log.stream}</h4>
            <button type="button" onClick={() => downloadLog(message.card.workflowId, log)} className="rounded border bg-white px-2 py-1 text-slate-700">下载 {log.stream} 脱敏日志</button>
          </div>
          {log.truncated && <p className="mt-1 text-amber-700">日志已截断；下载内容也是当前已保留的脱敏片段。</p>}
          <pre className="mt-1 max-h-64 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-2 text-slate-100">{log.text || '（无输出）'}</pre>
        </section>)}
        {diagnosisLogs.length === 0 && <p className="text-slate-600">本次失败没有可用的引擎输出。</p>}
      </div>
      <section className="mt-3 rounded bg-white p-2 text-slate-700" aria-label="大模型分析">
        <h4 className="font-medium text-slate-900">大模型分析</h4>
        {analysis ? <>
          <p className="mt-1">{analysis.summary}</p>
          <h5 className="mt-2 font-medium">已确认事实</h5>
          <ul className="list-disc pl-5">{analysis.facts.map((item, index) => <li key={index}>{item.statement}<code className="ml-1 text-slate-500">{item.evidence}</code></li>)}</ul>
          <h5 className="mt-2 font-medium">可能原因</h5>
          <ul className="list-disc pl-5">{analysis.possibleCauses.map((item, index) => <li key={index}>{item.statement}<code className="ml-1 text-slate-500">{item.evidence}</code></li>)}</ul>
          <h5 className="mt-2 font-medium">仍未知</h5>
          <ul className="list-disc pl-5">{analysis.unknowns.map((item, index) => <li key={index}>{item}</li>)}</ul>
          <h5 className="mt-2 font-medium">建议动作</h5>
          <ul className="list-disc pl-5">{analysis.recommendations.map((item, index) => <li key={index}>{item}</li>)}</ul>
        </> : <p className="mt-1 text-slate-500">智能分析暂不可用</p>}
      </section>
    </section>}
    {attempts.length > 0 && <section className="mt-2 text-xs">
      <button type="button" aria-expanded={attemptsExpanded} onClick={() => setAttemptsExpanded((value) => !value)} className="rounded border px-2 py-1">
        {['failed', 'service_restarted'].includes(state) ? '自动修正记录' : '执行尝试'}（{attempts.length}）
      </button>
      {attemptsExpanded && <div className="mt-2 space-y-2">
        {attempts.map((attempt) => {
          const parentAnalysis = attempt.parentAttemptId == null
            ? undefined
            : attempts.find((candidate) => candidate.id === attempt.parentAttemptId)?.failureAnalysis
          const repairBasis = attempt.phase === 'repair' ? parentAnalysis?.summary : attempt.failureAnalysis?.summary
          return <section key={attempt.id} className="rounded bg-slate-50 p-2">
          <p>第 {attempt.attemptNumber} 次 · {attempt.phase} · {attempt.status}{attempt.exitCode !== null && attempt.exitCode !== undefined ? ` · 退出码 ${attempt.exitCode}` : ''}</p>
          {attempt.scriptContentHash && <p className="mt-1 break-all text-slate-500">脚本 hash：{attempt.scriptContentHash}</p>}
          {attempt.scriptChangedFromParent === false && <p className="mt-1 font-medium text-amber-700">未产生实质修改</p>}
          {attempt.scriptChangedFromParent === true && <p className="mt-1 text-emerald-700">脚本内容已修改</p>}
          {repairBasis && <p className="mt-1 text-slate-600">依据：{repairBasis}</p>}
          {attempt.sanitizedStderr && <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-white p-2">{attempt.sanitizedStderr}</pre>}
          </section>
        })}
      </div>}
    </section>}
    {state === 'succeeded' && <div className="mt-3 text-xs">
      <p className="text-emerald-700">脚本已生成并执行成功。</p>
      {outputDatasetIds.length > 0 && <ul className="mt-2">{outputDatasetIds.map((id) => <li key={id}>结果数据集 #{id}</li>)}</ul>}
    </div>}
    {['failed', 'service_restarted', 'cancelled_by_user'].includes(state) && <div role="alert" className="mt-3 text-xs text-red-700">
      {state === 'cancelled_by_user' && <p>{failureText(workflow?.error ?? message.card.failureCode)}</p>}
      {scriptId && onRetry && <button type="button" disabled={retrying} onClick={() => void retry()} className="mt-2 rounded border border-red-200 bg-white px-3 py-1.5">{retrying ? '提交中…' : '重新尝试'}</button>}
    </div>}
    {!terminal.has(state) && <button type="button" disabled={cancelling} onClick={() => void stop()} className="mt-3 rounded border px-3 py-1.5 text-xs">{cancelling ? '取消中…' : '取消'}</button>}
  </article>
    {state === 'succeeded' && workflow?.runId && onOpenDataset && <WorkflowResultsCard key={workflow.runId} runId={String(workflow.runId)} onOpenDataset={onOpenDataset} />}
  </>
}
