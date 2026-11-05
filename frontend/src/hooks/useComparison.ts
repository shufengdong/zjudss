import { useCallback, useEffect, useState } from 'react'
import { getComparison } from '../api/client'
import type { RunComparison } from '../types'
import { useRequestGate } from './useRequestGate'

export function useComparison(id: string | null) {
  const [value, setValue] = useState<RunComparison | null>(null)
  const [loading, setLoading] = useState(Boolean(id))
  const [error, setError] = useState<string | null>(null)
  const { begin, isCurrent } = useRequestGate()
  const reload = useCallback(async () => {
    const ticket = begin()
    setValue(null)
    setError(null)
    setLoading(Boolean(id))
    if (!id) return null
    try {
      const result = await getComparison(id, ticket.signal)
      if (isCurrent(ticket)) setValue(result)
      return result
    } catch (cause) {
      if (isCurrent(ticket)) {
        setError(cause instanceof Error ? cause.message : '比较加载失败')
      }
      return null
    } finally {
      if (isCurrent(ticket)) setLoading(false)
    }
  }, [begin, id, isCurrent])

  useEffect(() => { void reload() }, [reload])
  return { value, loading, error, reload }
}
