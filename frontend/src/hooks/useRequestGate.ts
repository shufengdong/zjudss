import { useCallback, useEffect, useRef } from 'react'

export interface RequestTicket {
  generation: number
  signal: AbortSignal
}

export function useRequestGate() {
  const generation = useRef(0)
  const controller = useRef<AbortController | null>(null)

  const invalidate = useCallback(() => {
    generation.current += 1
    controller.current?.abort()
    controller.current = null
  }, [])

  const begin = useCallback((): RequestTicket => {
    invalidate()
    const next = new AbortController()
    controller.current = next
    return { generation: generation.current, signal: next.signal }
  }, [invalidate])

  const isCurrent = useCallback((ticket: RequestTicket) => (
    generation.current === ticket.generation && !ticket.signal.aborted
  ), [])

  useEffect(() => invalidate, [invalidate])
  return { begin, isCurrent, invalidate }
}
