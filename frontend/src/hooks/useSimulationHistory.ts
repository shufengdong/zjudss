import { useEffect, useRef, useState } from 'react'
import { listTasks } from '../api/client'
import type { SimulationTask } from '../types'

export function useSimulationHistory(id: string | null, revision = 0) {
  const [tasks, setTasks] = useState<SimulationTask[]>([])
  const generation = useRef(0)
  useEffect(() => {
    const request = ++generation.current
    if (!id) { setTasks([]); return }
    void listTasks(id).then(value => {
      if (generation.current === request) setTasks(value)
    })
  }, [id, revision])
  return tasks
}
