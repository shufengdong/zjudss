import type { ChartConfig } from '../../types'
import ChartPreview from './ChartPreview'
import PresentationControls from './chart_builder/PresentationControls'
import TemplateControls from './chart_builder/TemplateControls'
import { useChartBuilder } from './chart_builder/useChartBuilder'

interface ChartBuilderProps {
  datasetId: string
  sourceName: string
  onSaved: (viewId: string) => void
  notify: (text: string) => void
  initialConfig?: ChartConfig
  initialName?: string
}

const reasonLabel = (reason: string | null | undefined): string => {
  if (reason === 'no_category_field') return '缺少分类字段'
  if (reason === 'no_numeric_field') return '缺少数值字段'
  if (reason === 'capability_truncated') return '字段分量过多'
  return '当前数据不兼容'
}

export default function ChartBuilder(props: ChartBuilderProps) {
  const builder = useChartBuilder(props)
  if (builder.capabilities.loading) {
    return <p className="p-4 text-sm text-slate-500">正在读取可视化字段…</p>
  }
  if (builder.capabilities.error) {
    return <div role="alert" className="m-4 text-sm text-red-600">
      {builder.capabilities.error}
      <button onClick={builder.capabilities.reload}>重试</button>
    </div>
  }
  if (!builder.capabilities.data || !builder.state) return null

  const template = builder.capabilities.data.templates.find(
    item => item.kind === builder.state?.kind,
  )
  return <div className="grid gap-4 p-4 lg:grid-cols-[minmax(280px,360px)_minmax(420px,1fr)]">
    <fieldset disabled={builder.saving} className="contents">
      <section className="grid content-start gap-3">
        <TemplateControls
          state={builder.state}
          capabilities={builder.capabilities.data}
          categories={builder.categories}
          measures={builder.measures}
          measureLimit={builder.measureLimit}
          displayedMeasureKeys={builder.displayedMeasureKeys}
          onPatch={builder.patch}
          onKindChange={builder.changeKind}
          onTitleChange={builder.changeTitle}
          onXAxisModeChange={builder.changeXAxisMode}
          onMeasureKeysChange={builder.changeMeasureKeys}
        />
        <PresentationControls state={builder.state} onPatch={builder.patch} />
        {!template?.enabled && <p className="text-sm text-amber-600">
          {reasonLabel(template?.reason)}
        </p>}
        <div className="flex gap-2">
          <button
            className="rounded bg-brand-600 px-3 py-2 text-sm text-white disabled:opacity-50"
            disabled={!builder.config || builder.preview.loading}
            onClick={builder.runPreview}
          >生成预览</button>
          <button
            className="rounded border px-3 py-2 text-sm disabled:opacity-50"
            disabled={!builder.fresh || builder.saving || !builder.state.name.trim()}
            onClick={() => void builder.save()}
          >保存图表</button>
        </div>
      </section>
    </fieldset>
    <ChartPreview
      model={builder.preview.model}
      loading={builder.preview.loading}
      error={builder.preview.error}
      stale={Boolean(builder.preview.model && !builder.fresh)}
    />
  </div>
}
