import { useCallback, useEffect, useState } from 'react'
import { getChartCapabilities } from '../api/client'
import type { ChartCapabilities } from '../types'
import { useRequestGate } from './useRequestGate'

export function useChartCapabilities(datasetId: string | null) {
  const [data, setData] = useState<ChartCapabilities | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [errorDatasetId, setErrorDatasetId] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  const { begin, isCurrent } = useRequestGate()
  const reload = useCallback(() => setRevision(value => value + 1), [])
  useEffect(() => {
    const request = begin()
    setData(null); setError(null); setErrorDatasetId(null); setLoading(Boolean(datasetId))
    if (!datasetId) return
    void getChartCapabilities(datasetId, request.signal).then(value => {
      if (isCurrent(request)) setData(value)
    }).catch(value => {
      if (isCurrent(request)) { setError(value instanceof Error ? value.message : '图表字段加载失败');setErrorDatasetId(datasetId) }
    }).finally(() => { if (isCurrent(request)) setLoading(false) })
  }, [begin, datasetId, isCurrent, revision])
  const visibleData = data?.datasetId === datasetId ? data : null
  return { data: visibleData, loading: loading || Boolean(datasetId && data && !visibleData), error: errorDatasetId === datasetId ? error : null, reload }
}
