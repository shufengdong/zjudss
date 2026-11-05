import { useComparison } from '../../hooks/useComparison'

export default function ComparisonView({ id, onDataset }: { id: string; onDataset: (id: string) => void }) {
  const { value, loading, error, reload } = useComparison(id)
  if (error) return <div role="alert" className="space-y-2 p-4 text-sm text-red-700"><p>{error}</p><button type="button" onClick={() => void reload()} className="rounded border px-3 py-1.5">重试加载比较</button></div>
  if (loading || !value) return <p className="p-4 text-sm text-slate-500">加载中…</p>
  return <div className="p-4"><h2 className="font-semibold">运行 {value.baselineRunId} 与 {value.candidateRunId}</h2>{value.datasets.map(d=><button key={d.id} onClick={()=>onDataset(d.id)} className="mt-2 block rounded border p-2 text-sm">{d.name}</button>)}</div>
}
