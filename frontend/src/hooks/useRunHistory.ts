import { useCallback, useEffect, useRef, useState } from 'react'
import { deleteRun as deleteRunApi, listRuns, previewRunDeletion } from '../api/client'
import type { RunDeletionPreview, SimulationRun } from '../types'

export interface RunHistoryState {
  runs: SimulationRun[]
  status: 'idle' | 'loading' | 'ready' | 'error'
  error: string | null
  reload: () => void
  previewDeletion:(id:string)=>Promise<RunDeletionPreview>
  deleteRun:(id:string,impactToken:string)=>Promise<void>
}

function errorMessage(error: unknown): string {
  return error instanceof Error && error.message.trim()
    ? error.message
    : '运行记录加载失败'
}

export function useRunHistory(id: string | null, revision = 0): RunHistoryState {
  const [runs, setRuns] = useState<SimulationRun[]>([])
  const [status, setStatus] = useState<RunHistoryState['status']>(id ? 'loading' : 'idle')
  const [error, setError] = useState<string | null>(null)
  const [reloadRevision, setReloadRevision] = useState(0)
  const generation = useRef(0)
  const reload = useCallback(() => setReloadRevision((value) => value + 1), [])
  const previewDeletion=useCallback((runId:string)=>previewRunDeletion(runId),[])
  const deleteRun=useCallback(async(runId:string,impactToken:string)=>{await deleteRunApi(runId,impactToken);setRuns(values=>values.filter(run=>run.id!==runId))},[])

  useEffect(() => {
    const request = ++generation.current
    if (!id) {
      setRuns([])
      setStatus('idle')
      setError(null)
      return
    }

    setRuns([])
    setStatus('loading')
    setError(null)
    void listRuns(id).then(
      (value) => {
        if (generation.current !== request) return
        setRuns(value)
        setStatus('ready')
      },
      (cause: unknown) => {
        if (generation.current !== request) return
        setError(errorMessage(cause))
        setStatus('error')
      },
    )
  }, [id, revision, reloadRevision])

  return { runs, status, error, reload, previewDeletion, deleteRun }
}
