import { useCallback, useEffect, useRef, useState } from 'react'
import { listRunDatasets } from '../../api/client'
import type { RunHistoryState } from '../../hooks/useRunHistory'
import type { DatasetMetadata, SimulationRun } from '../../types'
import type { RunDeletionPreview } from '../../types'
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
  runDurationMs,
  runFailure,
  runStageLabel,
  runStatusLabel,
  runTitle,
} from './runPresentation'

type DatasetDetail = {
  status: 'idle' | 'loading' | 'ready' | 'error'
  data: DatasetMetadata[]
  error: string | null
}

const EMPTY_DETAIL: DatasetDetail = { status: 'idle', data: [], error: null }

function requestError(error: unknown): string {
  return error instanceof Error && error.message.trim() ? error.message : '未知错误'
}

function statusClass(status: SimulationRun['status']): string {
  if (status === 'succeeded') return 'bg-emerald-50 text-emerald-700'
  if (status === 'failed') return 'bg-red-50 text-red-700'
  if (status === 'running') return 'bg-blue-50 text-blue-700'
  return 'bg-slate-100 text-slate-600'
}

function RunTimestamps({ run }: { run: SimulationRun }) {
  return <div className="mt-2 space-y-1 text-slate-500">
    <p>创建时间：{formatTimestamp(run.createdAt)}</p>
    <p>开始时间：{formatTimestamp(run.startedAt)}</p>
    <p>结束时间：{formatTimestamp(run.finishedAt)}</p>
  </div>
}

function RunDetails({ run, detail, now, onRetry, onOpenDataset }: {
  run: SimulationRun
  detail: DatasetDetail
  now: number
  onRetry: () => void
  onOpenDataset: (datasetId: string) => void
}) {
  const duration = formatDuration(runDurationMs(run, now))
  if (run.status === 'failed') {
    const failure = runFailure(run)
    return <div role="alert" className="space-y-1 border-t border-red-100 bg-red-50/60 px-3 py-3 text-xs text-red-800">
      <p>失败阶段：{runStageLabel(run.stage)}</p>
      <p className="break-words">失败原因：{failure.message}</p>
      {failure.code && <p className="break-all">错误码：{failure.code}</p>}
      {failure.engineOutput && <div className="space-y-1">
        <p>tensoreval 报错：</p>
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded border border-red-200 bg-white p-2 font-mono text-[11px] text-red-900">{failure.engineOutput}</pre>
      </div>}
      {run.exitCode !== null && <p>退出码：{run.exitCode}</p>}
      <p>耗时：{duration}</p>
      <RunTimestamps run={run} />
    </div>
  }
  if (run.status === 'queued' || run.status === 'running') {
    return <div className="space-y-1 border-t px-3 py-3 text-xs text-slate-600">
      <p>当前阶段：{runStageLabel(run.stage)}</p>
      <p>{run.status === 'queued' ? '已等待' : '已运行'}：{duration}</p>
      <p className="text-slate-400">计算完成后可查看结果</p>
      <RunTimestamps run={run} />
    </div>
  }
  if (detail.status === 'loading') {
    return <div aria-live="polite" className="border-t px-3 py-4 text-xs text-slate-500">正在加载计算结果…</div>
  }
  if (detail.status === 'error') {
    return <div role="alert" className="space-y-2 border-t bg-red-50/60 px-3 py-3 text-xs text-red-700">
      <p className="break-words">结果加载失败：{detail.error}</p>
      <button type="button" onClick={onRetry} className="rounded border border-red-200 bg-white px-2 py-1">重试加载计算结果</button>
    </div>
  }
  return <div className="border-t px-3 py-3 text-xs">
    {detail.data.length === 0
      ? <p className="text-slate-500">本次运行没有可用结果，结果可能已被删除</p>
      : <>
        <h3 className="font-medium text-slate-700">计算结果（{detail.data.length}）</h3>
        <ul className="mt-2 space-y-1.5">
          {detail.data.map((dataset) => <li key={dataset.id} className="flex min-w-0 items-center gap-2 rounded bg-slate-50 px-2 py-2">
            <span className="min-w-0 flex-1 truncate font-medium text-slate-700">{dataset.name}</span>
            <span className="shrink-0 text-slate-400">{dataset.rowCount} 行</span>
            <span className="shrink-0 text-slate-400">{formatBytes(dataset.sizeBytes)}</span>
            <button type="button" onClick={() => onOpenDataset(dataset.id)} className="shrink-0 rounded border bg-white px-2 py-1 text-brand-600" aria-label={`查看 ${dataset.name}`}>查看</button>
          </li>)}
        </ul>
      </>}
    <p className="mt-3 break-words text-slate-400">运行 #{run.id} · {run.scriptResourceId ? `脚本 #${run.scriptResourceId}` : '脚本尚未生成'} · Engine {run.engineContractVersion}</p>
    <RunTimestamps run={run} />
  </div>
}

export default function RunHistory({ history, datasetRevision, onOpenDataset, onLocateScript, onRunDeleted }: {
  history: RunHistoryState
  datasetRevision: number
  onOpenDataset: (datasetId: string) => void
  onLocateScript?:(messageId:string)=>Promise<void>|void
  onRunDeleted?:(runId:string,resultIds:string[])=>void
}) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const expandedRef = useRef(expanded)
  const [details, setDetails] = useState<Record<string, DatasetDetail>>({})
  const generations = useRef<Record<string, number>>({})
  const revision = useRef(datasetRevision)
  const previousStatuses = useRef<Record<string, SimulationRun['status']>>({})
  const hasLiveRuns = history.runs.some((run) => run.status === 'queued' || run.status === 'running')
  const [now, setNow] = useState(() => Date.now())
  const[deletion,setDeletion]=useState<{run:SimulationRun;preview:RunDeletionPreview}|null>(null),[deleting,setDeleting]=useState(false),[deleteError,setDeleteError]=useState<string|null>(null)
  const dialogRef=useRef<HTMLDivElement|null>(null),cancelDeleteRef=useRef<HTMLButtonElement|null>(null)

  useEffect(()=>{
    if(!deletion)return
    if(!deleting)cancelDeleteRef.current?.focus()
    const onKeyDown=(event:KeyboardEvent)=>{
      if(event.key==='Escape'&&!deleting){event.preventDefault();setDeletion(null);return}
      if(event.key!=='Tab')return
      const controls=Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])')??[])
      if(controls.length===0)return
      const first=controls[0],last=controls[controls.length-1]
      if(event.shiftKey&&document.activeElement===first){event.preventDefault();last.focus()}
      else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first.focus()}
    }
    document.addEventListener('keydown',onKeyDown)
    return()=>document.removeEventListener('keydown',onKeyDown)
  },[deletion,deleting])

  useEffect(() => {
    if (!hasLiveRuns) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(timer)
  }, [hasLiveRuns])

  const loadDatasets = useCallback((runId: string) => {
    const generation = (generations.current[runId] ?? 0) + 1
    generations.current[runId] = generation
    setDetails((current) => ({
      ...current,
      [runId]: { status: 'loading', data: current[runId]?.data ?? [], error: null },
    }))
    void listRunDatasets(runId).then(
      (data) => {
        if (generations.current[runId] !== generation || !expandedRef.current.has(runId)) return
        setDetails((current) => ({ ...current, [runId]: { status: 'ready', data, error: null } }))
      },
      (error: unknown) => {
        if (generations.current[runId] !== generation || !expandedRef.current.has(runId)) return
        setDetails((current) => ({
          ...current,
          [runId]: { status: 'error', data: current[runId]?.data ?? [], error: requestError(error) },
        }))
      },
    )
  }, [])

  useEffect(() => {
    if (revision.current === datasetRevision) return
    revision.current = datasetRevision
    for (const runId of Object.keys(generations.current)) {
      generations.current[runId] += 1
    }
    setDetails((current) => Object.fromEntries(
      Object.entries(current).map(([runId, detail]) => [
        runId,
        { ...detail, status: 'idle', data: [], error: null },
      ]),
    ))
    for (const run of history.runs) {
      if (run.status === 'succeeded' && expandedRef.current.has(run.id)) loadDatasets(run.id)
    }
  }, [datasetRevision, history.runs, loadDatasets])

  useEffect(() => {
    const nextStatuses: Record<string, SimulationRun['status']> = {}
    for (const run of history.runs) {
      nextStatuses[run.id] = run.status
      if (
        run.status === 'succeeded'
        && previousStatuses.current[run.id] !== 'succeeded'
        && expandedRef.current.has(run.id)
      ) loadDatasets(run.id)
    }
    previousStatuses.current = nextStatuses
  }, [history.runs, loadDatasets])

  const toggle = (run: SimulationRun) => {
    const next = new Set(expandedRef.current)
    if (next.has(run.id)) {
      next.delete(run.id)
      generations.current[run.id] = (generations.current[run.id] ?? 0) + 1
      setDetails((current) => current[run.id]?.status === 'loading'
        ? { ...current, [run.id]: { ...current[run.id], status: 'idle' } }
        : current)
    } else {
      next.add(run.id)
      const detail = details[run.id]
      if (run.status === 'succeeded' && (!detail || detail.status === 'idle')) loadDatasets(run.id)
    }
    expandedRef.current = next
    setExpanded(next)
  }
  const previewDelete=async(run:SimulationRun)=>{setDeleteError(null);try{setDeletion({run,preview:await history.previewDeletion(run.id)})}catch(cause){setDeleteError(requestError(cause))}}
  const confirmDelete=async()=>{if(!deletion)return;setDeleting(true);setDeleteError(null);try{await history.deleteRun(deletion.run.id,deletion.preview.impactToken);onRunDeleted?.(deletion.run.id,deletion.preview.resultIds.map(String));setDeletion(null)}catch(cause){setDeleteError(requestError(cause))}finally{setDeleting(false)}}

  if (history.status === 'loading') {
    return <div aria-live="polite" className="p-4 text-xs text-slate-500">正在加载运行记录…</div>
  }
  if (history.status === 'error') {
    return <div role="alert" className="m-3 space-y-3 rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-red-700">
      <p>运行记录加载失败：{history.error ?? '未知错误'}</p>
      <button type="button" onClick={history.reload} className="rounded border border-red-200 bg-white px-2 py-1">重试加载运行记录</button>
    </div>
  }
  return <div className="space-y-2 p-3">
    {history.runs.map((run) => {
      const isExpanded = expanded.has(run.id)
      const detailId = `run-details-${run.id}`
      const summary=<>
          <span className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-sm font-semibold text-slate-800">{runTitle(run)}</span>
            <span className={`shrink-0 rounded px-2 py-0.5 ${statusClass(run.status)}`}>{runStatusLabel(run.status)}</span>
          </span>
          <span className="mt-1 block truncate text-slate-500">{run.scriptName} · 模型版本 {run.modelVersionId}</span>
          <span className="mt-1 flex items-center text-slate-400">
            <span>耗时 {formatDuration(runDurationMs(run, now))} · {formatTimestamp(run.createdAt)}</span>
            {!onLocateScript&&<span className="ml-auto pl-2 text-brand-600">{isExpanded ? '收起详情' : '展开详情'}</span>}
          </span>
        </>
      return <article key={run.id} className="overflow-hidden rounded-lg border bg-white text-xs">
        <div className="flex items-stretch">{onLocateScript?<button type="button" disabled={!run.scriptMessageId} onClick={()=>run.scriptMessageId?void onLocateScript(run.scriptMessageId):undefined} className="min-w-0 flex-1 px-3 py-3 text-left disabled:cursor-default">{summary}{!run.scriptMessageId&&<span className="mt-1 block text-amber-600">脚本尚未生成，无法定位</span>}</button>:<button type="button" aria-expanded={isExpanded} aria-controls={detailId} onClick={()=>toggle(run)} className="block min-w-0 flex-1 px-3 py-3 text-left">{summary}</button>}{onLocateScript&&<button type="button" aria-label={isExpanded?'收起运行详情':'展开运行详情'} aria-expanded={isExpanded} aria-controls={detailId} onClick={()=>toggle(run)} className="border-l px-3 text-brand-600">{isExpanded?'收起':'详情'}</button>}{['succeeded','failed'].includes(run.status)&&<button type="button" aria-label={`删除运行 ${run.id}`} onClick={()=>void previewDelete(run)} className="border-l px-3 text-red-600">删除</button>}</div>
        {isExpanded && <div id={detailId}>
          <RunDetails
            run={run}
            detail={details[run.id] ?? EMPTY_DETAIL}
            now={now}
            onRetry={() => loadDatasets(run.id)}
            onOpenDataset={onOpenDataset}
          />
        </div>}
      </article>
    })}
    {history.status === 'ready' && history.runs.length === 0 && <p className="text-xs text-slate-400">暂无运行记录</p>}
    {deleteError&&<p role="alert" className="rounded bg-red-50 p-2 text-xs text-red-700">{deleteError}</p>}
    {deletion&&<div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-4"><div ref={dialogRef} role="dialog" aria-modal="true" aria-label="删除运行记录" aria-describedby="run-deletion-impact" className="w-full max-w-md rounded-xl bg-white p-5 shadow-2xl"><h2 className="font-semibold">删除运行记录</h2><div id="run-deletion-impact"><p className="mt-2 text-sm">将删除运行 #{deletion.run.id} 及 {deletion.preview.resultCount} 个直接结果（{formatBytes(deletion.preview.resultBytes)}）。</p><p className="mt-2 text-xs text-slate-600">{deletion.preview.derivedReferenceCount} 个派生引用、{deletion.preview.promotedReferenceCount} 个提升资源将保留墓碑血缘。</p></div><div className="mt-4 flex justify-end gap-2"><button ref={cancelDeleteRef} type="button" disabled={deleting} onClick={()=>setDeletion(null)} className="rounded border px-3 py-1.5">取消</button><button type="button" disabled={deleting} onClick={()=>void confirmDelete()} className="rounded bg-red-600 px-3 py-1.5 text-white">{deleting?'删除中…':'确认删除'}</button></div></div></div>}
  </div>
}
