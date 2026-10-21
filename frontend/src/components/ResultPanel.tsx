import type { CurveData, ResultTab } from '../types'
import LineChart from './charts/LineChart'
import { XIcon } from './icons'

interface ResultPanelProps {
  open: boolean
  activeTab: ResultTab
  overlay: boolean
  onTabChange: (tab: ResultTab) => void
  onClose: () => void
  voltageCurve: CurveData
  powerCurve: CurveData
}

const TABS: { key: ResultTab; label: string }[] = [
  { key: 'curves', label: '结果曲线' },
  { key: 'report', label: '仿真报告' },
  { key: 'logs', label: '运行日志' },
]

type LogLevel = 'info' | 'warn' | 'error'

interface LogEntry {
  time: string
  level: LogLevel
  text: string
}

const LOGS: LogEntry[] = [
  { time: '09:12:03', level: 'info', text: 'inspect_model 完成：model ok' },
  { time: '09:12:05', level: 'info', text: 'run_simulation 提交任务 sim_20261021_001' },
  { time: '09:12:06', level: 'info', text: '时序仿真启动，步长 15min，时长 24h' },
  { time: '09:14:21', level: 'info', text: '时序仿真完成，共 96 个时步' },
  { time: '09:14:22', level: 'warn', text: '检测到 3 个节点电压越限（< 0.95 p.u.）' },
  { time: '09:14:30', level: 'info', text: '结果分析完成' },
  { time: '09:14:31', level: 'info', text: '报告生成完成' },
]

const LEVEL_CLS: Record<LogLevel, string> = {
  info: 'text-slate-400',
  warn: 'text-amber-600',
  error: 'text-red-600',
}

function PanelBody({
  activeTab,
  voltageCurve,
  powerCurve,
}: {
  activeTab: ResultTab
  voltageCurve: CurveData
  powerCurve: CurveData
}) {
  if (activeTab === 'curves') {
    return (
      <div className="space-y-5 p-4">
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-700">24 小时电压曲线（两条馈线）</h3>
          <LineChart data={voltageCurve} height={280} />
        </section>
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-700">24 小时有功功率曲线</h3>
          <LineChart data={powerCurve} height={280} />
        </section>
      </div>
    )
  }

  if (activeTab === 'report') {
    return (
      <div className="space-y-4 p-4 text-sm leading-relaxed text-slate-600">
        <h3 className="text-base font-semibold text-slate-800">城区配电网 24 小时时序仿真报告</h3>
        <div className="rounded-lg bg-slate-50 p-3">
          <div className="text-xs font-medium text-slate-400">任务编号</div>
          <div className="font-mono text-xs text-slate-700">sim_20261021_001</div>
        </div>
        <section>
          <h4 className="mb-1 font-medium text-slate-700">关键指标</h4>
          <ul className="list-inside list-disc space-y-1">
            <li>最大电压：1.082 p.u.（≤ 1.10 限值内）</li>
            <li>最小电压：0.941 p.u.（低于 0.95 限值，越限）</li>
            <li>越限节点：3 个</li>
            <li>累计越限时长：2.5 h</li>
          </ul>
        </section>
        <section>
          <h4 className="mb-1 font-medium text-slate-700">结论与建议</h4>
          <p>
            越限集中在光伏出力退出后的晚高峰（18:00–21:00），末端节点电压跌落明显。建议在节点
            N-23 增配无功补偿，或调整主变分接头以提升电压水平。
          </p>
        </section>
      </div>
    )
  }

  return (
    <div className="p-4">
      <div className="space-y-1.5 font-mono text-xs">
        {LOGS.map((l, i) => (
          <div key={i} className="flex gap-2">
            <span className="shrink-0 text-slate-400">{l.time}</span>
            <span className={`shrink-0 ${LEVEL_CLS[l.level]}`}>[{l.level}]</span>
            <span className="text-slate-600">{l.text}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

export default function ResultPanel(props: ResultPanelProps) {
  const { open, activeTab, overlay, onTabChange, onClose, voltageCurve, powerCurve } = props

  const header = (
    <div className="flex h-11 shrink-0 items-center gap-1 border-b border-slate-200 px-2">
      {TABS.map((t) => (
        <button
          key={t.key}
          type="button"
          onClick={() => onTabChange(t.key)}
          className={`relative px-2.5 py-2 text-sm transition-colors ${
            activeTab === t.key ? 'font-medium text-brand-600' : 'text-slate-500 hover:text-slate-700'
          }`}
        >
          {t.label}
          {activeTab === t.key && <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-brand-500" />}
        </button>
      ))}
      <button
        type="button"
        onClick={onClose}
        className="ml-auto rounded-md p-1.5 text-slate-400 hover:bg-slate-100 hover:text-slate-600"
        title="收起面板"
      >
        <XIcon className="h-4 w-4" />
      </button>
    </div>
  )

  if (overlay) {
    if (!open) return null
    return (
      <>
        <button type="button" className="fixed inset-0 z-30 bg-black/30" onClick={onClose} aria-label="关闭结果面板" />
        <aside className="fixed inset-y-0 right-0 z-40 flex w-[380px] max-w-[92vw] flex-col border-l border-slate-200 bg-white shadow-2xl">
          {header}
          <div className="flex-1 overflow-y-auto">
            <PanelBody activeTab={activeTab} voltageCurve={voltageCurve} powerCurve={powerCurve} />
          </div>
        </aside>
      </>
    )
  }

  return (
    <aside
      className={`${open ? 'w-[380px]' : 'w-0'} shrink-0 overflow-hidden border-l border-slate-200 bg-white transition-[width] duration-200`}
    >
      <div className="flex h-full w-[380px] flex-col">
        {header}
        <div className="flex-1 overflow-y-auto">
          <PanelBody activeTab={activeTab} voltageCurve={voltageCurve} powerCurve={powerCurve} />
        </div>
      </div>
    </aside>
  )
}
