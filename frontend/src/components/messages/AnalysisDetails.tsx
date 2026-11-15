import { useEffect, useMemo, useRef, useState } from 'react'
import { getAnalysis } from '../../api/client'
import type { AnalysisInputSnapshot, AnalysisRun } from '../../types'

type LoadState =
  | { kind: 'idle' | 'loading' }
  | { kind: 'ready'; details: AnalysisRun }
  | { kind: 'error'; message: string }

function sourceSummary(input: AnalysisInputSnapshot) {
  switch (input.sourceKind) {
    case 'active_model':
      return `active 模型 · ${input.sourceName} · alias ${input.alias} · modelDataframeId ${input.modelDataframeId} · modelVersionId ${input.modelVersionId}`
    case 'dataset':
      return `结果数据集 · ${input.sourceName} · alias ${input.alias} · datasetId ${input.datasetId}`
    case 'data_resource':
      return `本地数据 · ${input.sourceName} · alias ${input.alias} · resourceId ${input.resourceId}`
  }
}

export default function AnalysisDetails({ analysisRunId, notify }: {
  analysisRunId: string
  notify: (message: string) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const [revision, setRevision] = useState(0)
  const [state, setState] = useState<LoadState>({ kind: 'idle' })
  const generation = useRef(0)

  useEffect(() => {
    if (!expanded) return
    const current = ++generation.current
    const controller = new AbortController()
    setState({ kind: 'loading' })
    void getAnalysis(analysisRunId, controller.signal).then(details => {
      if (generation.current === current && !controller.signal.aborted) {
        setState({ kind: 'ready', details })
      }
    }).catch(cause => {
      if (generation.current !== current || controller.signal.aborted) return
      const message = cause instanceof Error ? cause.message : '分析详情读取失败'
      setState({ kind: 'error', message })
      notify(message)
    })
    return () => {
      controller.abort()
      if (generation.current === current) generation.current += 1
    }
  }, [analysisRunId, expanded, notify, revision])

  const inputs = useMemo(() => state.kind === 'ready'
    ? [...state.details.inputs].sort((left, right) => left.ordinal - right.ordinal)
    : [], [state])

  return <div className="mt-3 border-t border-slate-200 pt-3 text-xs">
    <button type="button" className="text-brand-600 hover:text-brand-700" onClick={() => setExpanded(value => !value)}>
      {expanded ? '收起分析脚本与输入' : '查看分析脚本与输入'}
    </button>
    {expanded && <div className="mt-3 space-y-3">
      {state.kind === 'loading' && <p role="status" className="text-slate-500">正在读取分析详情…</p>}
      {state.kind === 'error' && <div role="alert" className="rounded bg-red-50 p-3 text-red-700">
        <p>{state.message}</p>
        <button type="button" className="mt-2 underline" onClick={() => setRevision(value => value + 1)}>重试读取分析详情</button>
      </div>}
      {state.kind === 'ready' && <>
        <div>
          <div className="mb-1 font-medium text-slate-700">只读 RSS 脚本</div>
          <pre aria-label="分析 RSS 脚本" className="max-h-64 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-3 font-mono text-slate-100">{state.details.script.content}</pre>
          <div className="mt-1 break-all font-mono text-slate-400">contentHash {state.details.script.contentHash}</div>
        </div>
        <div>
          <div className="mb-1 font-medium text-slate-700">冻结输入（按执行顺序）</div>
          <ol className="space-y-1">
            {inputs.map(input => <li data-testid="analysis-input" key={`${input.ordinal}-${input.alias}`} className="rounded bg-slate-50 p-2 text-slate-600">
              <div>{sourceSummary(input)}</div>
              <div className="mt-1 break-all font-mono text-[10px] text-slate-400">schemaHash {input.sourceSchemaHash} · contentHash {input.sourceContentHash}</div>
            </li>)}
          </ol>
        </div>
        <div className="text-slate-400">engine {state.details.engineContractVersion} · snapshot {state.details.engineResourceSnapshotId}</div>
      </>}
    </div>}
  </div>
}
