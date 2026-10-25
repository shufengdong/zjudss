import type { ResultColumn } from '../../types'

export interface DataFramePageShape {
  datasetId?: string
  resourceId?: string
  columns: string[]
  rows: unknown[][]
  nextCursor: string | null
  diagnostics: Record<string, number>
}

const show = (value: unknown) => value === null || value === undefined
  ? '—'
  : typeof value === 'object' ? JSON.stringify(value) : String(value)

export default function DataFrameTable({ columns, page, loading, onMore }: {
  columns: ResultColumn[]
  page: DataFramePageShape | null
  loading: boolean
  onMore: () => void
}) {
  const visible = page ? columns.filter(column => page.columns.includes(column.name)) : columns
  const names = visible.map(column => column.name)
  const indices = names.map(name => page?.columns.indexOf(name) ?? -1)
  return <div className="overflow-auto">
    <table className="min-w-full border-collapse text-xs">
      <thead className="sticky top-0 bg-slate-100"><tr>{visible.map(column =>
        <th key={column.name} className="whitespace-nowrap border-b px-3 py-2 text-left">
          {column.name}{column.unit ? <span className="text-slate-400"> ({column.unit})</span> : null}
        </th>,
      )}</tr></thead>
      <tbody>{page?.rows.map((row, rowIndex) =>
        <tr key={rowIndex} className="odd:bg-white even:bg-slate-50">{indices.map((index, columnIndex) =>
          <td key={names[columnIndex]} className="max-w-72 truncate border-b px-3 py-2">
            {show(index < 0 ? null : row[index])}
          </td>,
        )}</tr>,
      )}</tbody>
    </table>
    {page?.nextCursor && <button type="button" disabled={loading} onClick={onMore} className="m-3 rounded-lg border px-3 py-1.5 text-xs">
      {loading ? '加载中…' : '加载更多'}
    </button>}
  </div>
}
