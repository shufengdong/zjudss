import { useEffect, useRef, useState } from 'react'
import { ApiError, getChartViewData } from '../api/client'
import type { ChartModel } from '../types'

export function useChartView(id: string | null) {
  const [data, setData] = useState<ChartModel | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const generation = useRef(0)
  useEffect(() => {
    const request = ++generation.current
    const controller = new AbortController()
    setData(null)
    setError(null)
    setLoading(Boolean(id))
    if (!id) return () => controller.abort()
    void getChartViewData(id, controller.signal).then(value => {
      if (generation.current === request) setData(value)
    }).catch(value => {
      if (generation.current !== request || controller.signal.aborted) return
      const detail = value instanceof ApiError && value.details ? `（${JSON.stringify(value.details)}）` : ''
      setError((value instanceof Error ? value.message : '图表加载失败') + detail)
    }).finally(() => {
      if (generation.current === request) setLoading(false)
    })
    return () => controller.abort()
  }, [id])
  return { data, error, loading }
}
