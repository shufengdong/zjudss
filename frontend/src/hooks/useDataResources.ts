import { useCallback, useEffect, useRef, useState } from 'react'
import { listDataResources } from '../api/client'
import { mergeByStableId } from '../state/mergeByStableId'
import type { DataResourceView } from '../types'

export function useDataResources(sessionId: string | null) {
  const [resources, setResources] = useState<DataResourceView[]>([])
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)

  const load = useCallback(async (cursor?: string, append = false) => {
    if (!sessionId) return
    const current = append ? generation.current : ++generation.current
    setLoading(true)
    setError(null)
    try {
      const page = await listDataResources(sessionId, { cursor, limit: 50 })
      if (generation.current !== current) return
      setResources(previous => append ? mergeByStableId(previous, page.resources) : page.resources)
      setNextCursor(page.nextCursor)
    } catch (cause) {
      if (generation.current === current) setError(cause instanceof Error ? cause.message : '数据文件加载失败')
    } finally {
      if (generation.current === current) setLoading(false)
    }
  }, [sessionId])

  useEffect(() => {
    generation.current += 1
    setResources([])
    setNextCursor(null)
    setError(null)
    if (sessionId) void load()
    return () => { generation.current += 1 }
  }, [sessionId, load])

  const reload = useCallback(() => load(), [load])
  const loadMore = useCallback(() => nextCursor ? load(nextCursor, true) : Promise.resolve(), [load, nextCursor])
  return { resources, loading, error, nextCursor, reload, loadMore }
}
