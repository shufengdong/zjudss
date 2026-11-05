import { useEffect, useState } from 'react'
import { ApiError, getChartViewData } from '../api/client'
import type { ChartModel } from '../types'
import { useRequestGate } from './useRequestGate'

export function useChartView(id: string | null) {
  const [data, setData] = useState<ChartModel | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const { begin, isCurrent } = useRequestGate()
  useEffect(() => {
    const request = begin()
    setData(null)
    setError(null)
    setLoading(Boolean(id))
    if (!id) return
    void getChartViewData(id, request.signal).then(value => {
      if (isCurrent(request)) setData(value)
    }).catch(value => {
      if (!isCurrent(request)) return
      const detail = value instanceof ApiError && value.details ? `（${JSON.stringify(value.details)}）` : ''
      setError((value instanceof Error ? value.message : '图表加载失败') + detail)
    }).finally(() => {
      if (isCurrent(request)) setLoading(false)
    })
  }, [begin, id, isCurrent])
  return { data, error, loading }
}
