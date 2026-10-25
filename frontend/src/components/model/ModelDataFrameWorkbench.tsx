import { useEffect, useState } from 'react'
import { useModelDataFrame } from '../../hooks/useModelDataFrame'
import type { ModelDataFrameSource, Workspace } from '../../types'
import DataFrameTable from '../dataframes/DataFrameTable'

function titleFor(source: ModelDataFrameSource, activeVersionId?: string | null) {
  if (source.kind === 'model-draft') return `可变草稿 · 修订 ${source.revision} · ${source.tableName}`
  const label = source.versionId === activeVersionId ? '当前模型' : '历史版本'
  return `${label} · 版本 ${source.versionId} · ${source.tableName}`
}

export default function ModelDataFrameWorkbench({
  source,
  activeVersionId,
  onClose,
  onSourceChange,
  reloadWorkspace,
  notify,
}: {
  source: ModelDataFrameSource
  activeVersionId?: string | null
  onClose: () => void
  onSourceChange: (source: ModelDataFrameSource) => void
  reloadWorkspace: () => Promise<Workspace | undefined>
  notify: (message: string) => void
}) {
  const [selectedColumns, setSelectedColumns] = useState<string[] | null>(null)
  const [columnsOpen, setColumnsOpen] = useState(false)
  const { metadata, page, loading, error, conflict, loadMore, download } = useModelDataFrame(
    source,
    selectedColumns ?? undefined,
  )
  const title = titleFor(source, activeVersionId)
  useEffect(() => setSelectedColumns(null), [source.kind, source.dataframeId])

  const toggleColumn = (name: string) => {
    if (!metadata) return
    const all = metadata.columns.map(column => column.name)
    const current = selectedColumns ?? all
    const next = current.includes(name)
      ? current.filter(column => column !== name)
      : all.filter(column => column === name || current.includes(column))
    if (next.length > 0) setSelectedColumns(next.length === all.length ? null : next)
  }
  const save = async () => {
    const file = await download()
    if (!file) return
    const url = URL.createObjectURL(file.blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = file.filename
    anchor.click()
    URL.revokeObjectURL(url)
  }
  const refreshDraft = async () => {
    if (source.kind !== 'model-draft') return
    const workspace = await reloadWorkspace()
    const draft = workspace?.draft
    const table = draft?.tables.find(value => value.tableName === source.tableName)
    if (!draft || !table) {
      notify(`草稿中的 ${source.tableName} 表已不存在`)
      onClose()
      return
    }
    onSourceChange({
      kind: 'model-draft',
      dataframeId: table.id,
      draftId: draft.id,
      revision: draft.revision,
      tableName: table.tableName,
    })
  }

  return <section
    role="dialog"
    aria-modal="true"
    aria-label={title}
    className="flex h-full w-full min-w-0 flex-col bg-white sm:min-w-[560px]"
  >
    <header className="flex h-12 items-center gap-2 border-b px-3 sm:px-4">
      <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h2>
      {metadata && <div className="relative text-xs">
        <button type="button" aria-expanded={columnsOpen} onClick={() => setColumnsOpen(value => !value)} className="rounded border px-2 py-1">选择列</button>
        {columnsOpen && <div className="absolute right-0 z-10 mt-1 max-h-64 min-w-36 overflow-auto rounded border bg-white p-2 shadow">
          {metadata.columns.map(column => {
            const checked = selectedColumns === null || selectedColumns.includes(column.name)
            return <label key={column.name} className="flex gap-2 whitespace-nowrap py-1">
              <input
                aria-label={`显示列 ${column.name}`}
                type="checkbox"
                checked={checked}
                disabled={checked && (selectedColumns?.length ?? metadata.columns.length) === 1}
                onChange={() => toggleColumn(column.name)}
              />
              {column.name}
            </label>
          })}
        </div>}
      </div>}
      <button type="button" onClick={() => void save()} className="rounded border px-2 py-1 text-xs">下载 Parquet</button>
      <button type="button" aria-label="关闭模型表" onClick={onClose}>×</button>
    </header>
    {error && <div role="alert" className="m-3 rounded bg-red-50 p-3 text-sm text-red-600">
      {conflict ? '草稿已更新。已加载数据仍可查看。' : `${error}。已加载数据仍可查看，可重试。`}
      {conflict && <div className="mt-2 flex gap-2">
        <button type="button" onClick={() => void refreshDraft()} className="rounded border bg-white px-2 py-1">重新加载最新修订</button>
        <button type="button" onClick={onClose} className="rounded border bg-white px-2 py-1">关闭</button>
      </div>}
    </div>}
    {!metadata && loading && <p className="p-4 text-sm text-slate-500">正在加载模型表…</p>}
    {metadata && <div className="min-h-0 flex-1 overflow-auto">
      <DataFrameTable columns={metadata.columns} page={page} loading={loading} onMore={() => void loadMore()}/>
    </div>}
  </section>
}
