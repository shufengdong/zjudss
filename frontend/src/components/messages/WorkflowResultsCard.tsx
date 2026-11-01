import { useEffect, useState } from 'react'
import { listRunDatasets } from '../../api/client'
import type { DatasetMetadata } from '../../types'
import ResultCard from './ResultCard'

export default function WorkflowResultsCard({ runId, onOpenDataset }: {
  runId: string
  onOpenDataset: (datasetId: string) => void
}) {
  const [datasets, setDatasets] = useState<DatasetMetadata[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  useEffect(() => {
    let active = true
    setDatasets(null)
    setError(null)
    void listRunDatasets(runId).then(value => {
      if (active) setDatasets(value)
    }).catch(cause => {
      if (active) setError(cause instanceof Error ? cause.message : '结果读取失败')
    })
    return () => { active = false }
  }, [runId, revision])

  if (error) return <div role="alert" className="mx-4 my-2 rounded-xl border p-4 text-sm">
    <p>{error}</p>
    <button type="button" onClick={() => setRevision(value => value + 1)}>重试读取结果</button>
  </div>
  if (!datasets) return <p role="status" className="mx-4 my-2 text-sm">正在读取运行结果…</p>
  if (datasets.length === 0) return <p className="mx-4 my-2 text-sm">本次运行暂无可用结果数据集</p>
  return <ResultCard handlers={{ onOpenDataset }} message={{
    id: `workflow-results-${runId}`, type: 'result', timestamp: '', runId, title: '运行结果',
    outputs: datasets.map(dataset => ({
      datasetId: dataset.id, name: dataset.name, rowCount: dataset.rowCount, columns: dataset.columns,
      previewTruncated: false, defaultViewId: dataset.views.find(view => view.isDefault)?.id ?? null,
      views: dataset.views,
    })),
  }} />
}
