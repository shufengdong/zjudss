import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import type { CurveData, Message, MessageHandlers, ResultTab, Session } from './types'
import { useMediaQuery } from './hooks/useMediaQuery'
import {
  createSession,
  deleteSession as deleteSessionApi,
  fetchCurves,
  fetchReport,
  listMessages,
  listSessions,
  postMessage,
  renameSession as renameSessionApi,
  uploadAttachment,
} from './api/client'
import { subscribeEvents } from './api/events'
import { initialMessageState, messageReducer } from './state/messageReducer'
import Sidebar from './components/Sidebar'
import Composer from './components/Composer'
import ResultPanel from './components/ResultPanel'
import MessageItem from './components/messages/MessageItem'
import { ChevronDownIcon, PanelLeftIcon, PanelRightIcon } from './components/icons'

// 乐观消息的临时 id 前缀：发送时先本地展示，等后端经 SSE 回传真实 id 后替换。
const LOCAL_USER = 'local-user-'

function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [messageState, dispatchMessages] = useReducer(messageReducer, initialMessageState)
  const messages = messageState.messages
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [panelOpen, setPanelOpen] = useState(false)
  const [activeTab, setActiveTab] = useState<ResultTab>('curves')
  const [draft, setDraft] = useState('')
  const [toast, setToast] = useState<string | null>(null)

  // 结果面板数据：按 taskId 拉取真实曲线 / 报告。
  const [resultTaskId, setResultTaskId] = useState<string | null>(null)
  const [curves, setCurves] = useState<CurveData | null>(null)
  const [curvesLoading, setCurvesLoading] = useState(false)
  const [curvesError, setCurvesError] = useState<string | null>(null)
  const [reportText, setReportText] = useState<string | null>(null)
  const [reportFilename, setReportFilename] = useState<string | null>(null)
  const [reportLoading, setReportLoading] = useState(false)
  const [reportError, setReportError] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)

  const isNarrow = useMediaQuery('(max-width: 1279px)')

  const toastTimer = useRef<number | undefined>(undefined)
  const selectedIdRef = useRef<string | null>(null)
  const closeEventsRef = useRef<(() => void) | null>(null)

  const notify = useCallback((text: string) => {
    setToast(text)
    if (toastTimer.current) window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(null), 2200)
  }, [])

  const closeEvents = useCallback(() => {
    closeEventsRef.current?.()
    closeEventsRef.current = null
  }, [])

  const reloadMessages = useCallback(async (id: string) => {
    const list = await listMessages(id)
    dispatchMessages({ type: 'snapshot', sessionId: id, messages: list })
  }, [])

  // 先打开 SSE，再读取快照；reducer 会合并快照期间到达的实时事件。
  const openEvents = useCallback(
    (id: string) => {
      closeEvents()
      const subscription = subscribeEvents(id, {
        onMessage: (message) => {
          dispatchMessages({ type: 'event', sessionId: id, message })
        },
        onControl: () => {
          dispatchMessages({ type: 'select-session', sessionId: id })
          void reloadMessages(id).catch(() => notify('事件游标已失效，重新加载消息失败'))
        },
      })
      closeEventsRef.current = subscription.close
      return subscription.opened
    },
    [closeEvents, notify, reloadMessages],
  )

  const resetPanel = useCallback(() => {
    setResultTaskId(null)
    setCurves(null)
    setCurvesError(null)
    setReportText(null)
    setReportFilename(null)
    setReportError(null)
    setReportLoading(false)
    setCurvesLoading(false)
  }, [])

  const selectSession = useCallback(
    async (id: string) => {
      if (selectedIdRef.current === id) return
      selectedIdRef.current = id
      setSelectedId(id)
      dispatchMessages({ type: 'select-session', sessionId: id })
      setPanelOpen(false)
      resetPanel()
      try {
        await openEvents(id)
        await reloadMessages(id)
        if (selectedIdRef.current !== id) return // 已切换走，丢弃过期回放
      } catch (err) {
        if (selectedIdRef.current === id) {
          notify(err instanceof Error ? err.message : '加载消息失败')
        }
      }
    },
    [notify, openEvents, reloadMessages, resetPanel],
  )

  const newSession = useCallback(async () => {
    try {
      const s = await createSession('新的仿真')
      setSessions((prev) => [s, ...prev])
      selectedIdRef.current = s.id
      setSelectedId(s.id)
      dispatchMessages({ type: 'select-session', sessionId: s.id })
      setPanelOpen(false)
      resetPanel()
      setActiveTab('curves')
      void openEvents(s.id).catch(() => notify('事件流连接失败'))
    } catch (err) {
      notify(err instanceof Error ? err.message : '新建会话失败')
    }
  }, [notify, openEvents, resetPanel])

  const renameSession = async (id: string, title: string) => {
    const prev = sessions
    setSessions((list) => list.map((s) => (s.id === id ? { ...s, title } : s)))
    try {
      const updated = await renameSessionApi(id, title)
      setSessions((list) => list.map((s) => (s.id === updated.id ? updated : s)))
    } catch (err) {
      setSessions(prev)
      notify(err instanceof Error ? err.message : '重命名失败')
    }
  }

  const deleteSession = async (id: string) => {
    const remaining = sessions.filter((s) => s.id !== id)
    setSessions(remaining)
    if (selectedId === id) {
      const next = remaining[0] ?? null
      closeEvents()
      selectedIdRef.current = null
      setSelectedId(null)
      dispatchMessages({ type: 'select-session', sessionId: null })
      setPanelOpen(false)
      resetPanel()
      if (next) void selectSession(next.id)
    }
    try {
      await deleteSessionApi(id)
    } catch (err) {
      notify(err instanceof Error ? err.message : '删除失败')
      // 删除失败：重拉会话列表，恢复被乐观移除的会话。
      void listSessions().then(setSessions).catch(() => {})
    }
  }

  const send = () => {
    const text = draft.trim()
    if (!text || !selectedId) return
    const ts = new Date().toISOString()
    const optimistic: Message = {
      id: `${LOCAL_USER}${Date.now()}`,
      type: 'user',
      timestamp: ts,
      content: text,
    }
    const prevDraft = draft

    dispatchMessages({ type: 'optimistic', sessionId: selectedId, message: optimistic })
    setDraft('')

    void postMessage(selectedId, text).catch((err) => {
      notify(err instanceof Error ? err.message : '发送失败')
      dispatchMessages({ type: 'remove-optimistic', sessionId: selectedId, id: optimistic.id })
      setDraft(prevDraft)
    })
  }

  // 附件上传：multipart → 201 返回 attachment 消息；SSE 也会广播同一条（appendEvent 按 id 去重）。
  const uploadFiles = async (files: File[]) => {
    if (!selectedId) return
    setUploading(true)
    try {
      for (const file of files) {
        try {
          const msg = await uploadAttachment(selectedId, file)
          dispatchMessages({ type: 'event', sessionId: selectedId, message: msg })
        } catch (err) {
          notify(err instanceof Error ? err.message : `上传 ${file.name} 失败`)
        }
      }
    } finally {
      setUploading(false)
    }
  }

  // 结果面板：按 taskId 拉真实曲线。
  const loadCurves = useCallback(async (taskId: string) => {
    setResultTaskId(taskId)
    setActiveTab('curves')
    setPanelOpen(true)
    setCurves(null)
    setCurvesError(null)
    setCurvesLoading(true)
    try {
      setCurves(await fetchCurves(taskId))
    } catch (err) {
      setCurvesError(err instanceof Error ? err.message : '加载曲线失败')
    } finally {
      setCurvesLoading(false)
    }
  }, [])

  // 结果面板：按 taskId 拉真实报告（Markdown 文本）。
  const loadReport = useCallback(async (taskId: string) => {
    setResultTaskId(taskId)
    setReportText(null)
    setReportFilename(null)
    setReportError(null)
    setReportLoading(true)
    try {
      const { blob, filename } = await fetchReport(taskId)
      setReportFilename(filename)
      setReportText(await blob.text())
    } catch (err) {
      setReportError(err instanceof Error ? err.message : '加载报告失败')
    } finally {
      setReportLoading(false)
    }
  }, [])

  const downloadReport = useCallback(
    async (taskId: string) => {
      try {
        const { blob, filename } = await fetchReport(taskId)
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = filename
        document.body.appendChild(a)
        a.click()
        a.remove()
        URL.revokeObjectURL(url)
      } catch (err) {
        notify(err instanceof Error ? err.message : '下载报告失败')
      }
    },
    [notify],
  )

  const changeTab = useCallback(
    (tab: ResultTab) => {
      setActiveTab(tab)
      if (tab === 'report' && resultTaskId) void loadReport(resultTaskId)
    },
    [resultTaskId, loadReport],
  )

  // 初始：加载会话列表并选中第一条。
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const list = await listSessions()
        if (cancelled) return
        setSessions(list)
        if (list.length > 0) await selectSession(list[0].id)
      } catch (err) {
        if (!cancelled) notify(err instanceof Error ? err.message : '加载会话失败')
      }
    })()
    return () => {
      cancelled = true
    }
  }, [selectSession, notify])

  // 卸载时关闭 SSE。
  useEffect(() => {
    return () => closeEvents()
  }, [closeEvents])

  const handlers: MessageHandlers = {
    onViewCurves: (taskId) => {
      void loadCurves(taskId)
    },
    onDownloadReport: (taskId) => {
      void downloadReport(taskId)
    },
    notify,
  }

  const selected = sessions.find((s) => s.id === selectedId)
  const title = selected?.title ?? '新的仿真'

  return (
    <div className="flex h-screen w-full overflow-hidden">
      <Sidebar
        sessions={sessions}
        selectedId={selectedId}
        collapsed={sidebarCollapsed}
        onSelect={selectSession}
        onNew={newSession}
        onRename={renameSession}
        onDelete={deleteSession}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
      />

      <main className="flex min-w-0 flex-1 flex-col bg-slate-50">
        <header className="flex h-14 shrink-0 items-center gap-2 border-b border-slate-200 bg-white px-4">
          <button
            type="button"
            onClick={() => setSidebarCollapsed((v) => !v)}
            className="rounded-md p-1.5 text-slate-500 hover:bg-slate-100"
            title="切换侧栏"
          >
            <PanelLeftIcon className="h-5 w-5" />
          </button>
          <h1 className="truncate text-sm font-semibold text-slate-800">{title}</h1>
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick={() => setPanelOpen((v) => !v)}
              className="rounded-md p-1.5 text-slate-500 hover:bg-slate-100"
              title="结果面板"
            >
              <PanelRightIcon className="h-5 w-5" />
            </button>
            <span className="inline-flex items-center gap-1 rounded-full border border-slate-200 bg-slate-50 px-2.5 py-1 text-xs font-medium text-slate-600">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              DeepSeek
              <ChevronDownIcon className="h-3 w-3 text-slate-400" />
            </span>
          </div>
        </header>

        <div className="flex-1 overflow-y-auto">
          {messages.length === 0 ? (
            <div className="flex h-full items-center justify-center px-6 text-center">
              <div className="space-y-2">
                <div className="text-3xl">⚡</div>
                <p className="text-sm font-medium text-slate-500">开始一次新的配电网仿真</p>
                <p className="text-xs text-slate-400">输入仿真需求，或上传配电网模型文件</p>
              </div>
            </div>
          ) : (
            <div className="py-4">
              {messages.map((m) => (
                <MessageItem key={m.id} message={m} handlers={handlers} />
              ))}
            </div>
          )}
        </div>

        <Composer
          value={draft}
          onChange={setDraft}
          onSend={send}
          uploading={uploading}
          onUploadFiles={uploadFiles}
        />
      </main>

      <ResultPanel
        open={panelOpen}
        activeTab={activeTab}
        overlay={isNarrow}
        onTabChange={changeTab}
        onClose={() => setPanelOpen(false)}
        curves={curves}
        curvesLoading={curvesLoading}
        curvesError={curvesError}
        reportTaskId={resultTaskId}
        reportText={reportText}
        reportFilename={reportFilename}
        reportLoading={reportLoading}
        reportError={reportError}
        onDownloadReport={downloadReport}
      />

      {toast && (
        <div className="pointer-events-none fixed bottom-6 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-slate-900 px-4 py-2 text-sm text-white shadow-lg">
          {toast}
        </div>
      )}
    </div>
  )
}

export default App
