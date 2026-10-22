import { useCallback, useEffect, useRef, useState } from 'react'
import { getDataset, getDatasetRows } from '../api/client'
import type { DatasetMetadata, DatasetPage } from '../types'

export function mergePages(current: DatasetPage | null, next: DatasetPage): DatasetPage {
  if (!current) return next
  return { ...next, rows: [...current.rows, ...next.rows] }
}

export function useDataset(id: string | null, columns?: string[]) {
  const [metadata, setMetadata] = useState<DatasetMetadata | null>(null)
  const [page, setPage] = useState<DatasetPage | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)
  const inFlight = useRef<string | null>(null)
  const projection = columns?.join(',') || undefined

  useEffect(() => {
    const request = ++generation.current
    setMetadata(null)
    setPage(null)
    setError(null)
    setLoading(Boolean(id))
    inFlight.current = null
    if (!id) return
    void Promise.all([getDataset(id), getDatasetRows(id, { limit: 100, ...(projection ? { columns: projection } : {}) })])
      .then(([nextMetadata, nextPage]) => {
        if (generation.current !== request) return
        setMetadata(nextMetadata)
        setPage(nextPage)
      })
      .catch(value => {
        if (generation.current === request) setError(value instanceof Error ? value.message : '数据集加载失败')
      })
      .finally(() => {
        if (generation.current === request) setLoading(false)
      })
  }, [id, projection])

  const loadMore = useCallback(async () => {
    if (!id || !page?.nextCursor || inFlight.current === id) return
    const request = generation.current
    inFlight.current = id
    setLoading(true)
    setError(null)
    try {
      const next = await getDatasetRows(id, { limit: 100, cursor: page.nextCursor, ...(projection ? { columns: projection } : {}) })
      if (generation.current === request) setPage(current => mergePages(current, next))
    } catch (value) {
      if (generation.current === request) setError(value instanceof Error ? value.message : '数据集加载失败')
    } finally {
      if (generation.current === request) setLoading(false)
      if (inFlight.current === id) inFlight.current = null
    }
  }, [id, page?.nextCursor, projection])

  return { metadata, page, loading, error, loadMore }
}
