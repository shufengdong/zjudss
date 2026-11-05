import type { ChartPalette } from '../../../types'
import type { ChartBuilderState } from './state'

export default function PresentationControls({
  state,
  onPatch,
}: {
  state: ChartBuilderState
  onPatch: (patch: Partial<ChartBuilderState>) => void
}) {
  return <details>
    <summary className="cursor-pointer text-sm">显示设置</summary>
    <div className="grid gap-2 py-2">
      <label>调色板
        <select
          aria-label="调色板"
          value={state.palette}
          onChange={event => onPatch({ palette: event.target.value as ChartPalette })}
        >
          <option value="colorblind">色觉友好</option>
          <option value="default">默认</option>
          <option value="highContrast">高对比</option>
          <option value="monochrome">单色</option>
        </select>
      </label>
      <label>
        <input
          type="checkbox"
          checked={state.legend}
          onChange={event => onPatch({ legend: event.target.checked })}
        /> 显示图例
      </label>
      {state.legend && <>
        <label>图例位置
          <select
            aria-label="图例位置"
            value={state.legendPosition}
            onChange={event => onPatch({
              legendPosition: event.target.value as ChartBuilderState['legendPosition'],
            })}
          >
            <option value="auto">自动</option>
            <option value="top">顶部</option>
            <option value="bottom">底部</option>
            <option value="right">右侧</option>
          </select>
        </label>
        <label>图例方向
          <select
            aria-label="图例方向"
            value={state.legendOrientation}
            onChange={event => onPatch({
              legendOrientation: event.target.value as ChartBuilderState['legendOrientation'],
            })}
          >
            <option value="auto">自动</option>
            <option value="horizontal">横向</option>
            <option value="vertical">纵向</option>
          </select>
        </label>
      </>}
      <label>
        <input
          type="checkbox"
          checked={state.titleShow}
          onChange={event => onPatch({ titleShow: event.target.checked })}
        /> 显示标题
      </label>
      <label>
        <input
          type="checkbox"
          checked={state.grid}
          onChange={event => onPatch({ grid: event.target.checked })}
        /> 显示网格
      </label>
      {state.kind === 'line' && <>
        <label>线宽
          <input
            aria-label="线宽"
            type="number"
            min={1}
            max={4}
            value={state.lineWidth}
            onChange={event => onPatch({
              lineWidth: Number(event.target.value) as ChartBuilderState['lineWidth'],
            })}
          />
        </label>
        <label>线型
          <select
            aria-label="线型"
            value={state.lineStyle}
            onChange={event => onPatch({
              lineStyle: event.target.value as ChartBuilderState['lineStyle'],
            })}
          >
            <option value="solid">实线</option>
            <option value="dashed">虚线</option>
            <option value="dotted">点线</option>
          </select>
        </label>
        <label>
          <input
            type="checkbox"
            checked={state.showSymbol}
            onChange={event => onPatch({ showSymbol: event.target.checked })}
          /> 显示数据点
        </label>
      </>}
    </div>
  </details>
}
