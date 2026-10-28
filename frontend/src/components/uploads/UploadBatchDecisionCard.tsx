import { useMemo, useState } from 'react'
import { selectableCandidateIds } from '../../hooks/useUploadBatches'
import type { CandidateDecision, ReplacementAction, UploadBatchActions, UploadBatchView } from '../../types'

const defaultDecision = (candidate: UploadBatchView['candidates'][number]): CandidateDecision => ({
  candidateId: candidate.id,
  purpose: candidate.purpose ?? 'data_resource',
  selected: candidate.evidence?.hidden === true ? false : candidate.selected,
  tableName: candidate.proposedAlias ?? candidate.displayName,
  displayName: candidate.displayName,
  alias: candidate.proposedAlias,
})

export default function UploadBatchDecisionCard({ batch, actions, notify }: {
  batch: UploadBatchView
  actions?: UploadBatchActions
  notify: (text: string) => void
}) {
  const [decisions, setDecisions] = useState<CandidateDecision[]>(() => batch.candidates.map(defaultDecision))
  const [busy, setBusy] = useState(false)
  const selectable = useMemo(() => selectableCandidateIds(batch.candidates), [batch.candidates])
  const selected = decisions.filter((item) => item.selected && selectable.includes(item.candidateId)).length
  const act = async (work: () => Promise<unknown>) => {
    setBusy(true)
    try { await work() } catch (cause) {
      notify(cause instanceof Error ? cause.message : '批次操作失败')
    } finally { setBusy(false) }
  }
  const publish = async (replacementAction?: ReplacementAction) => {
    await actions!.updateCandidates(batch.id, decisions)
    await actions!.confirm(batch.id, decisions, replacementAction)
    notify(replacementAction === 'replace' ? '资源已覆盖' : replacementAction === 'save_as_new' ? '已另存为新资源' : replacementAction === 'cancel' ? '已取消替换' : '上传内容已确认')
  }
  const update = (id: string, patch: Partial<CandidateDecision>) => setDecisions((values) => values.map((value) => value.candidateId === id ? { ...value, ...patch } : value))

  return <section className="rounded-xl border border-blue-200 bg-blue-50 p-3 text-xs" aria-label={`上传批次 ${batch.id}`}>
    <div className="flex items-center"><h3 className="font-semibold">上传批次 #{batch.id}</h3><span className="ml-auto">{batch.status}</span></div>
    <p className="mt-1 text-slate-500">模型：{batch.modelPublishStatus} · 数据：{batch.resourcePublishStatus}</p>
    {batch.status === 'awaiting_parse_settings' && <button type="button" disabled={busy || !actions} onClick={() => void act(() => actions!.updateParseSettings(batch.id, { encoding: 'utf-8', headerRow: 0, decimal: '.' }))} className="mt-2 rounded border bg-white px-2 py-1">使用默认解析设置</button>}
    {batch.candidates.length > 0 && <label className="mt-2 flex gap-2 border-b border-blue-200 pb-2">
      <input type="checkbox" checked={selectable.length > 0 && selected === selectable.length} onChange={(event) => setDecisions((values) => values.map((value) => selectable.includes(value.candidateId) ? { ...value, selected: event.target.checked } : value))} />
      <span>全选可见工作表</span>
    </label>}
    {batch.candidates.map((candidate) => {
      const decision = decisions.find((value) => value.candidateId === candidate.id) ?? defaultDecision(candidate)
      const hidden = candidate.evidence?.hidden === true
      return <div key={candidate.id} className={`mt-2 rounded bg-white p-2 ${hidden ? 'opacity-60' : ''}`}>
        <div className="flex items-center gap-2">
          <input aria-label={`选择 ${candidate.displayName}`} type="checkbox" disabled={hidden} checked={decision.selected && !hidden} onChange={(event) => update(candidate.id, { selected: event.target.checked })} />
          <span className="min-w-0 flex-1 truncate">{candidate.displayName}{candidate.sheetName ? ` · ${candidate.sheetName}` : ''}</span>
          {hidden && <span>隐藏 sheet</span>}
        </div>
        <label className="mt-2 flex items-center gap-2">用途
          <select aria-label={`${candidate.displayName}用途`} value={decision.purpose} onChange={(event) => update(candidate.id, { purpose: event.target.value as CandidateDecision['purpose'] })} className="rounded border px-2 py-1">
            <option value="model_table">模型表</option><option value="data_resource">数据文件</option><option value="skip">跳过</option>
          </select>
        </label>
      </div>
    })}
    {batch.replacementResourceId && <pre className="mt-3 max-h-32 overflow-auto whitespace-pre-wrap rounded border border-amber-200 bg-amber-50 p-2">{JSON.stringify(batch.candidates.map((candidate) => candidate.evidence), null, 2)}</pre>}
    {['awaiting_purpose', 'ready_to_confirm', 'partially_confirmed', 'failed'].includes(batch.status) && (batch.replacementResourceId
      ? <div className="mt-3 flex flex-wrap gap-2">
        <button type="button" disabled={busy || !actions} onClick={() => void act(() => publish('replace'))} className="rounded bg-brand-500 px-3 py-1 text-white">覆盖当前资源</button>
        <button type="button" disabled={busy || !actions} onClick={() => void act(() => publish('save_as_new'))} className="rounded border bg-white px-3 py-1">另存为新资源</button>
        <button type="button" disabled={busy || !actions} onClick={() => void act(() => publish('cancel'))} className="rounded border bg-white px-3 py-1">取消替换</button>
      </div>
      : <div className="mt-3 flex gap-2">
        <button type="button" disabled={busy || !actions} onClick={() => void act(() => publish())} className="rounded bg-brand-500 px-3 py-1 text-white">确认用途并发布</button>
        <button type="button" disabled={busy || !actions} onClick={() => void act(() => actions!.discard(batch.id))} className="rounded border bg-white px-3 py-1">抛弃</button>
      </div>)}
  </section>
}
