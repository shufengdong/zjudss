import { useCallback, useEffect, useRef, useState } from 'react'
import { getChartCapabilities } from '../api/client'
import type { ChartCapabilities } from '../types'

export function useChartCapabilities(datasetId: string | null) {
  const [data, setData] = useState<ChartCapabilities | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [errorDatasetId, setErrorDatasetId] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  const generation = useRef(0)
  const reload = useCallback(() => setRevision(value => value + 1), [])
  useEffect(() => {
    const request = ++generation.current, controller = new AbortController()
    setData(null); setError(null); setErrorDatasetId(null); setLoading(Boolean(datasetId))
    if (!datasetId) return () => controller.abort()
    void getChartCapabilities(datasetId, controller.signal).then(value => {
      if (generation.current === request) setData(value)
    }).catch(value => {
      if (generation.current === request && !controller.signal.aborted) { setError(value instanceof Error ? value.message : '图表字段加载失败');setErrorDatasetId(datasetId) }
    }).finally(() => { if (generation.current === request) setLoading(false) })
    return () => controller.abort()
  }, [datasetId, revision])
  const visibleData = data?.datasetId === datasetId ? data : null
  return { data: visibleData, loading: loading || Boolean(datasetId && data && !visibleData), error: errorDatasetId === datasetId ? error : null, reload }
}
