import type { CurveData, ResultTab } from '../types'
import LineChart from './charts/LineChart'
import Markdown from './Markdown'
import { DownloadIcon, XIcon } from './icons'

interface ResultPanelProps {
  open: boolean
  activeTab: ResultTab
  overlay: boolean
  onTabChange: (tab: ResultTab) => void
  onClose: () => void
  curves: CurveData | null
  curvesLoading: boolean
  curvesError: string | null
  reportTaskId: string | null
  reportText: string | null
  reportFilename: string | null
  reportLoading: boolean
  reportError: string | null
  onDownloadReport: (taskId: string) => void
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

// 运行日志暂无后端端点，保留静态演示样例。
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

function Hint({ children }: { children: string }) {
  return <div className="px-4 py-8 text-center text-sm text-slate-400">{children}</div>
}

function Loading() {
  return <div className="px-4 py-8 text-center text-sm text-slate-400">加载中…</div>
}

function ErrorBox({ message }: { message: string }) {
  return <div className="m-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600">{message}</div>
}

function PanelBody(props: ResultPanelProps) {
  const {
    activeTab,
    curves,
    curvesLoading,
    curvesError,
    reportTaskId,
    reportText,
    reportFilename,
    reportLoading,
    reportError,
    onDownloadReport,
  } = props

  if (activeTab === 'curves') {
    if (curvesLoading) return <Loading />
    if (curvesError) return <ErrorBox message={curvesError} />
    if (!curves) {
      return <Hint>先在对话里点击某次仿真结果的「查看曲线」。</Hint>
    }
    return (
      <div className="space-y-5 p-4">
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-700">
            {curves.variable}曲线（{curves.series.length} 条馈线）
          </h3>
          <LineChart data={curves} height={280} />
        </section>
      </div>
    )
  }

  if (activeTab === 'report') {
    if (!reportTaskId) {
      return <Hint>先在对话里点击某次仿真结果的「下载报告」，或先查看曲线。</Hint>
    }
    if (reportLoading) return <Loading />
    if (reportError) return <ErrorBox message={reportError} />
    if (reportText === null) {
      return <Hint>该任务暂无报告。</Hint>
    }
    return (
      <div className="space-y-4 p-4">
        <div className="flex items-center gap-2">
          <h3 className="min-w-0 flex-1 truncate text-sm font-semibold text-slate-800">
            {reportFilename ?? '仿真报告'}
          </h3>
          <button
            type="button"
            onClick={() => onDownloadReport(reportTaskId)}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-600 transition-colors hover:bg-slate-50"
          >
            <DownloadIcon className="h-3.5 w-3.5" />
            下载
          </button>
        </div>
        <Markdown>{reportText}</Markdown>
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
  const { open, activeTab, overlay, onTabChange, onClose } = props

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
            <PanelBody {...props} />
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
          <PanelBody {...props} />
        </div>
      </div>
    </aside>
  )
}
