import { useCallback, useEffect, useRef, useState } from 'react'
import { getRun } from '../api/client'
import type { FailureDiagnosis } from '../types'
import { parseRunFailureDiagnosis } from '../components/runs/runPresentation'

interface RunFailureDiagnosisState {
  diagnosis: FailureDiagnosis | null
  loading: boolean
  error: string | null
}

const POLL_INTERVAL_MS = 750

export function useRunFailureDiagnosis(runId: string | null, enabled: boolean) {
  const [requestRevision, setRequestRevision] = useState(0)
  const [state, setState] = useState<RunFailureDiagnosisState>({
    diagnosis: null,
    loading: false,
    error: null,
  })
  const generation = useRef(0)
  const retry = useCallback(() => setRequestRevision((value) => value + 1), [])

  useEffect(() => {
    const currentGeneration = ++generation.current
    let cancelled = false
    let pollTimer: number | undefined
    const active = () => !cancelled && generation.current === currentGeneration

    if (!enabled || !runId) {
      setState({ diagnosis: null, loading: false, error: null })
      return () => { cancelled = true }
    }

    setState({ diagnosis: null, loading: true, error: null })
    const load = async (background: boolean) => {
      try {
        const run = await getRun(runId)
        if (!active()) return
        const diagnosis = parseRunFailureDiagnosis(run)
        setState({ diagnosis, loading: false, error: null })
        if (diagnosis.analysisStatus === 'pending') {
          pollTimer = window.setTimeout(() => { void load(true) }, POLL_INTERVAL_MS)
        }
      } catch (cause) {
        if (!active()) return
        setState((current) => ({
          diagnosis: background ? current.diagnosis : null,
          loading: false,
          error: cause instanceof Error ? cause.message : '运行失败详情读取失败',
        }))
      }
    }
    void load(false)

    return () => {
      cancelled = true
      if (pollTimer !== undefined) window.clearTimeout(pollTimer)
    }
  }, [enabled, requestRevision, runId])

  return { ...state, retry }
}
