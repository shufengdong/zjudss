import { useEffect, useRef, useState } from 'react'
import { ApiError, getDatasetView } from '../api/client'
import type { DatasetViewData } from '../types'

export function useDatasetView(id: string | null) {
  const [data, setData] = useState<DatasetViewData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)
  useEffect(() => {
    const request = ++generation.current
    setData(null)
    setError(null)
    if (!id) return
    void getDatasetView(id).then(value => {
      if (generation.current === request) setData(value)
    }).catch(value => {
      if (generation.current !== request) return
      const detail = value instanceof ApiError && value.details ? `（${JSON.stringify(value.details)}）` : ''
      setError((value instanceof Error ? value.message : '视图加载失败') + detail)
    })
  }, [id])
  return { data, error }
}
