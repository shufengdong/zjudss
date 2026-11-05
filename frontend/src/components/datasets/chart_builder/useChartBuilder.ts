import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createChartView } from '../../../api/client'
import { useChartCapabilities } from '../../../hooks/useChartCapabilities'
import { chartConfigFingerprint, useChartPreview } from '../../../hooks/useChartPreview'
import type { ChartConfig, ChartKind } from '../../../types'
import { chartFieldOptions } from '../ChartFieldSelect'
import {
  buildChartConfig,
  chartMeasureLimit,
  displayedMeasureKeys,
  seedChartBuilderState,
  type ChartBuilderState,
} from './state'

interface UseChartBuilderArgs {
  datasetId: string
  sourceName: string
  onSaved: (viewId: string) => void
  notify: (text: string) => void
  initialConfig?: ChartConfig
  initialName?: string
}

export function useChartBuilder({
  datasetId,
  sourceName,
  onSaved,
  notify,
  initialConfig,
  initialName,
}: UseChartBuilderArgs) {
  const capabilities = useChartCapabilities(datasetId)
  const preview = useChartPreview(datasetId)
  const [state, setState] = useState<ChartBuilderState | null>(null)
  const [saving, setSaving] = useState(false)
  const operation = useRef<{ requestKey: string; id: string } | null>(null)
  const seeded = useRef('')
  const categories = useMemo(
    () => capabilities.data ? chartFieldOptions(capabilities.data, 'category') : [],
    [capabilities.data],
  )
  const measures = useMemo(
    () => capabilities.data ? chartFieldOptions(capabilities.data, 'measure') : [],
    [capabilities.data],
  )
  const measureLimit = capabilities.data ? chartMeasureLimit(capabilities.data) : 16

  useEffect(() => {
    if (!capabilities.data) return
    const seedKey = JSON.stringify({
      datasetId,
      schemaHash: capabilities.data.schemaHash,
      sourceName,
      initialConfig: initialConfig ?? null,
      initialName: initialName ?? null,
    })
    if (seeded.current === seedKey) return
    seeded.current = seedKey
    setState(seedChartBuilderState({
      capabilities: capabilities.data,
      categories,
      measures,
      sourceName,
      initialConfig,
      initialName,
    }))
    operation.current = null
    preview.clear()
  }, [
    capabilities.data,
    categories,
    datasetId,
    initialConfig,
    initialName,
    measures,
    preview.clear,
    sourceName,
  ])

  const patch = useCallback((value: Partial<ChartBuilderState>) => {
    setState(current => current ? { ...current, ...value } : current)
  }, [])

  const changeKind = useCallback((kind: ChartKind) => {
    setState(current => {
      if (!current) return current
      let aggregation = current.aggregation
      if (kind === 'heatmap' && aggregation === 'none') aggregation = 'mean'
      if (kind !== 'heatmap' && aggregation === 'count') aggregation = 'mean'
      const xKey = kind !== 'histogram' && kind !== 'scatter'
        ? categories.find(option => !option.disabled)?.key ?? ''
        : current.xKey
      return { ...current, kind, aggregation, xKey }
    })
    preview.clear()
  }, [categories, preview.clear])

  const changeTitle = useCallback((title: string) => {
    patch({ title, titleShow: Boolean(title) })
  }, [patch])

  const changeXAxisMode = useCallback((xAxisMode: 'category' | 'value') => {
    const options = xAxisMode === 'value' ? measures : categories
    patch({
      xAxisMode,
      xKey: options.find(option => !option.disabled)?.key ?? '',
    })
  }, [categories, measures, patch])

  const changeMeasureKeys = useCallback((measureKeys: string[]) => {
    patch({ measureKeys: measureKeys.slice(0, measureLimit) })
  }, [measureLimit, patch])

  const config = useMemo(
    () => state && capabilities.data
      ? buildChartConfig(state, capabilities.data, categories, measures)
      : null,
    [capabilities.data, categories, measures, state],
  )
  const fingerprint = config ? chartConfigFingerprint(config) : null
  const fresh = Boolean(
    fingerprint
      && preview.previewHash === fingerprint
      && preview.model,
  )

  const runPreview = useCallback(() => {
    if (config) void preview.run(config)
  }, [config, preview.run])

  const save = useCallback(async () => {
    if (!state || !config || !fresh || !fingerprint) return
    const requestKey = JSON.stringify({ name: state.name, fingerprint })
    if (operation.current?.requestKey !== requestKey) {
      operation.current = { requestKey, id: crypto.randomUUID() }
    }
    setSaving(true)
    try {
      const response = await createChartView(datasetId, {
        clientOperationId: operation.current.id,
        name: state.name,
        config,
      })
      notify('图表已保存')
      onSaved(response.view.id)
    } catch (value) {
      notify(value instanceof Error ? value.message : '图表保存失败')
    } finally {
      setSaving(false)
    }
  }, [config, datasetId, fingerprint, fresh, notify, onSaved, state])

  return {
    capabilities,
    preview,
    state,
    patch,
    changeKind,
    changeTitle,
    changeXAxisMode,
    changeMeasureKeys,
    categories,
    measures,
    measureLimit,
    displayedMeasureKeys: state ? displayedMeasureKeys(state) : [],
    config,
    fresh,
    saving,
    runPreview,
    save,
  }
}
