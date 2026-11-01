import type { EngineLog, FailureDiagnosis } from '../../types'

function safeDownloadPrefix(value: string): string {
  const safe = value.trim().replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '')
  return safe || 'failure-diagnosis'
}

function downloadLog(downloadPrefix: string, log: EngineLog) {
  const blob = new Blob([log.text], { type: 'text/plain;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `${safeDownloadPrefix(downloadPrefix)}-${log.stream}-sanitized.log`
  anchor.click()
  URL.revokeObjectURL(url)
}

export default function FailureDiagnosisPanel({ diagnosis, downloadPrefix }: {
  diagnosis: FailureDiagnosis
  downloadPrefix: string
}) {
  const metadata = [
    diagnosis.code ? `错误码 ${diagnosis.code}` : null,
    diagnosis.stage ? `阶段 ${diagnosis.stage}` : null,
    diagnosis.exitCode !== null ? `退出码 ${diagnosis.exitCode}` : null,
    diagnosis.durationMs !== null ? `${(diagnosis.durationMs / 1_000).toFixed(1)} 秒` : null,
  ].filter((item): item is string => item !== null)

  return <section className="mt-3 rounded-lg border border-red-200 bg-red-50 p-3 text-xs" aria-label="失败诊断">
    <h3 className="font-semibold text-red-800">失败诊断</h3>
    <p className="mt-1 text-red-700">{diagnosis.message}</p>
    {metadata.length > 0 && <p className="mt-2 text-slate-600">{metadata.join(' · ')}</p>}
    <div className="mt-3 space-y-3">
      {diagnosis.engineLogs.map((log) => <section key={log.stream}>
        <div className="flex items-center gap-2">
          <h4 className="font-medium text-slate-800">引擎 {log.stream}</h4>
          <button type="button" onClick={() => downloadLog(downloadPrefix, log)} className="rounded border bg-white px-2 py-1 text-slate-700">
            下载 {log.stream} 脱敏日志
          </button>
        </div>
        {log.truncated && <p className="mt-1 text-amber-700">日志已截断；下载内容也是当前已保留的脱敏片段。</p>}
        <pre className="mt-1 max-h-64 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-2 text-slate-100">{log.text || '（无输出）'}</pre>
      </section>)}
      {diagnosis.engineLogs.length === 0 && <p className="text-slate-600">本次失败没有可用的引擎输出。</p>}
    </div>
    <section className="mt-3 rounded bg-white p-2 text-slate-700" aria-label="大模型分析">
      <h4 className="font-medium text-slate-900">大模型分析</h4>
      {diagnosis.analysisStatus === 'pending'
        ? <p className="mt-1 text-slate-500">正在生成智能分析</p>
        : diagnosis.analysisStatus === 'complete' && diagnosis.analysis
          ? <>
              <p className="mt-1">{diagnosis.analysis.summary}</p>
              <h5 className="mt-2 font-medium">已确认事实</h5>
              <ul className="list-disc pl-5">{diagnosis.analysis.facts.map((item, index) => <li key={index}>{item.statement}<code className="ml-1 text-slate-500">{item.evidence}</code></li>)}</ul>
              <h5 className="mt-2 font-medium">可能原因</h5>
              <ul className="list-disc pl-5">{diagnosis.analysis.possibleCauses.map((item, index) => <li key={index}>{item.statement}<code className="ml-1 text-slate-500">{item.evidence}</code></li>)}</ul>
              <h5 className="mt-2 font-medium">仍未知</h5>
              <ul className="list-disc pl-5">{diagnosis.analysis.unknowns.map((item, index) => <li key={index}>{item}</li>)}</ul>
              <h5 className="mt-2 font-medium">建议动作</h5>
              <ul className="list-disc pl-5">{diagnosis.analysis.recommendations.map((item, index) => <li key={index}>{item}</li>)}</ul>
            </>
          : <p className="mt-1 text-slate-500">智能分析暂不可用</p>}
    </section>
  </section>
}
