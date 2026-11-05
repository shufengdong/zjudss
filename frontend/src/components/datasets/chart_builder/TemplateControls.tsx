import type { ChartCapabilities, ChartFieldRef, ChartKind } from '../../../types'
import ChartFieldSelect, { type ChartFieldOption } from '../ChartFieldSelect'
import { fieldName, resolveField } from './fieldMapping'
import type { ChartBuilderState } from './state'

const kinds: { value: ChartKind; label: string }[] = [
  { value: 'line', label: '折线图' },
  { value: 'bar', label: '柱状图' },
  { value: 'scatter', label: '散点图' },
  { value: 'heatmap', label: '热力图' },
  { value: 'boxplot', label: '箱线图' },
  { value: 'histogram', label: '直方图' },
]

const configMeasures = (state: ChartBuilderState): ChartFieldRef[] => {
  const config = state.seedConfig
  if (!config) return []
  if (config.kind === 'line' || config.kind === 'bar' || config.kind === 'scatter') {
    return config.measures
  }
  return [config.value]
}

interface TemplateControlsProps {
  state: ChartBuilderState
  capabilities: ChartCapabilities
  categories: ChartFieldOption[]
  measures: ChartFieldOption[]
  measureLimit: number
  displayedMeasureKeys: string[]
  onPatch: (patch: Partial<ChartBuilderState>) => void
  onKindChange: (kind: ChartKind) => void
  onTitleChange: (title: string) => void
  onXAxisModeChange: (mode: 'category' | 'value') => void
  onMeasureKeysChange: (keys: string[]) => void
}

export default function TemplateControls({
  state,
  capabilities,
  categories,
  measures,
  measureLimit,
  displayedMeasureKeys,
  onPatch,
  onKindChange,
  onTitleChange,
  onXAxisModeChange,
  onMeasureKeysChange,
}: TemplateControlsProps) {
  const multiMeasure = state.kind === 'line' || state.kind === 'bar' || state.kind === 'scatter'
  const preservedMeasures = configMeasures(state)
  return <>
    <label className="grid gap-1 text-sm">图表类型
      <select
        aria-label="图表类型"
        value={state.kind}
        onChange={event => onKindChange(event.target.value as ChartKind)}
        className="rounded border p-2"
      >
        {kinds.map(item => {
          const status = capabilities.templates.find(template => template.kind === item.value)
          return <option key={item.value} value={item.value} disabled={!status?.enabled}>
            {item.label}{status?.enabled ? '' : '（不可用）'}
          </option>
        })}
      </select>
    </label>
    <label className="grid gap-1 text-sm">图表名称
      <input
        aria-label="图表名称"
        value={state.name}
        onChange={event => onPatch({ name: event.target.value })}
        className="rounded border p-2"
      />
    </label>
    <label className="grid gap-1 text-sm">标题
      <input
        aria-label="标题"
        value={state.title}
        onChange={event => onTitleChange(event.target.value)}
        className="rounded border p-2"
      />
    </label>
    {state.kind === 'scatter'
      ? <ChartFieldSelect
          label="X 数值"
          options={measures}
          value={state.numericXKey}
          onChange={value => onPatch({ numericXKey: value as string })}
        />
      : state.kind === 'histogram'
        ? null
        : <ChartFieldSelect
            label={state.kind === 'boxplot' ? '分类' : 'X 轴'}
            options={state.kind === 'line' && state.xAxisMode === 'value' ? measures : categories}
            value={state.xKey}
            onChange={value => onPatch({ xKey: value as string })}
          />}
    {state.kind === 'heatmap' && <ChartFieldSelect
      label="Y 轴"
      options={categories}
      value={state.secondCategoryKey}
      onChange={value => onPatch({ secondCategoryKey: value as string })}
    />}
    <ChartFieldSelect
      label={multiMeasure ? '数值列（可多选）' : '数值列'}
      options={measures}
      value={multiMeasure ? state.measureKeys : (state.measureKeys[0] ?? '')}
      multiple={multiMeasure}
      onChange={value => onMeasureKeysChange(
        (Array.isArray(value) ? value : [value]).slice(0, measureLimit),
      )}
    />
    <div className="grid gap-2">
      {displayedMeasureKeys.map(key => {
        const option = measures.find(item => item.key === key)
        if (!option) return null
        const field = resolveField(key, measures, preservedMeasures)
        const physical = fieldName(field)
        return <label key={key} className="grid gap-1 text-sm">{physical} 展示名
          <input
            aria-label={`${physical} 展示名`}
            value={state.measureLabels[key] ?? field.label ?? physical}
            maxLength={64}
            onChange={event => onPatch({
              measureLabels: { ...state.measureLabels, [key]: event.target.value },
            })}
            className="rounded border p-2"
          />
        </label>
      })}
    </div>
    {state.kind === 'histogram' && <label className="text-sm">分箱数
      <input
        aria-label="分箱数"
        type="number"
        min={1}
        max={200}
        value={state.bins}
        onChange={event => onPatch({ bins: Number(event.target.value) })}
      />
    </label>}
    {state.kind === 'boxplot' && <label className="text-sm">
      <input
        type="checkbox"
        checked={state.showOutliers}
        onChange={event => onPatch({ showOutliers: event.target.checked })}
      /> 显示离群点
    </label>}
    <details>
      <summary className="cursor-pointer text-sm">高级设置</summary>
      {(state.kind === 'line' || state.kind === 'bar' || state.kind === 'heatmap') &&
        <div className="grid gap-2 py-2">
          {state.kind === 'line' && <label>X 轴模式
            <select
              aria-label="X 轴模式"
              value={state.xAxisMode}
              onChange={event => onXAxisModeChange(event.target.value as 'category' | 'value')}
            >
              <option value="category">分类</option>
              <option value="value">数值</option>
            </select>
          </label>}
          <label>聚合
            <select
              aria-label="聚合"
              value={state.aggregation}
              onChange={event => onPatch({ aggregation: event.target.value as ChartBuilderState['aggregation'] })}
            >
              {state.kind !== 'heatmap' && <option value="none">不聚合</option>}
              <option value="mean">平均</option>
              <option value="min">最小</option>
              <option value="max">最大</option>
              <option value="sum">求和</option>
              {state.kind === 'heatmap' && <option value="count">计数</option>}
            </select>
          </label>
          {state.kind !== 'heatmap' && <label>顺序
            <select
              aria-label="顺序"
              value={state.order}
              onChange={event => onPatch({ order: event.target.value as ChartBuilderState['order'] })}
            >
              <option value="source">源顺序</option>
              <option value="xAsc">X 升序</option>
              <option value="xDesc">X 降序</option>
            </select>
          </label>}
          {state.kind === 'line' && state.xAxisMode === 'value' && <>
            <label>
              <input
                type="checkbox"
                checked={state.sampling}
                onChange={event => onPatch({ sampling: event.target.checked })}
              /> LTTB 采样
            </label>
            {state.sampling && <label>每系列点数
              <input
                aria-label="每系列点数"
                type="number"
                min={100}
                max={10000}
                value={state.samplePoints}
                onChange={event => onPatch({ samplePoints: Number(event.target.value) })}
              />
            </label>}
          </>}
        </div>}
    </details>
  </>
}
