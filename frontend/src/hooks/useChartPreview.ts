import { useCallback, useEffect, useRef, useState } from 'react'
import { previewChart } from '../api/client'
import type { ChartConfig, ChartModel } from '../types'

export const chartConfigFingerprint = (config: ChartConfig) => JSON.stringify(config)

export function useChartPreview(datasetId: string) {
  const [model, setModel] = useState<ChartModel | null>(null)
  const [previewHash, setPreviewHash] = useState<string | null>(null)
  const [resolvedDatasetId, setResolvedDatasetId] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadingDatasetId, setLoadingDatasetId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [errorDatasetId, setErrorDatasetId] = useState<string | null>(null)
  const generation = useRef(0), controller = useRef<AbortController | null>(null)
  const clear = useCallback(() => { ++generation.current; controller.current?.abort(); setModel(null);setPreviewHash(null);setResolvedDatasetId(null);setError(null);setErrorDatasetId(null);setLoading(false);setLoadingDatasetId(null) }, [])
  useEffect(() => clear, [datasetId, clear])
  const run = useCallback(async (config: ChartConfig) => {
    const request = ++generation.current
    controller.current?.abort(); controller.current = new AbortController()
    setLoading(true);setLoadingDatasetId(datasetId);setError(null);setErrorDatasetId(null);setModel(null);setPreviewHash(null);setResolvedDatasetId(null)
    try {
      const value = await previewChart(datasetId, config, controller.current.signal)
      if (generation.current === request) { setModel(value);setResolvedDatasetId(datasetId);setPreviewHash(chartConfigFingerprint(config)) }
    } catch (value) {
      if (generation.current === request && !controller.current.signal.aborted) { setError(value instanceof Error ? value.message : '图表预览失败');setErrorDatasetId(datasetId) }
    } finally { if (generation.current === request) setLoading(false) }
  }, [datasetId])
  const visible = resolvedDatasetId === datasetId
  return { model: visible ? model : null, previewHash: visible ? previewHash : null, loading: loading && loadingDatasetId === datasetId, error: errorDatasetId === datasetId ? error : null, run, clear }
}
