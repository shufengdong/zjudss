import { useCallback, useRef, useState } from 'react'
import { sessions as mockSessions } from './mocks/sessions'
import { demoMessages, messagesBySession } from './mocks/messages'
import { powerCurve, voltageCurve } from './mocks/curves'
import type { Attachment, Message, MessageHandlers, ResultTab, Session } from './types'
import { useMediaQuery } from './hooks/useMediaQuery'
import Sidebar from './components/Sidebar'
import Composer, { type PendingAttachment } from './components/Composer'
import ResultPanel from './components/ResultPanel'
import MessageItem from './components/messages/MessageItem'
import { ChevronDownIcon, PanelLeftIcon, PanelRightIcon } from './components/icons'

const MOCK_FILES: Attachment[] = [
  { fileName: '城区配电网模型.case', fileSize: '2.4 MB', fileType: 'case' },
  { fileName: '负荷曲线.csv', fileSize: '184 KB', fileType: 'csv' },
  { fileName: '拓扑校验报告.pdf', fileSize: '1.1 MB', fileType: 'pdf' },
]

const DEFAULT_SESSION_ID = mockSessions[0]?.id ?? ''

function App() {
  const [sessions, setSessions] = useState<Session[]>(mockSessions)
  const [selectedId, setSelectedId] = useState<string | null>(DEFAULT_SESSION_ID)
  const [messages, setMessages] = useState<Message[]>(messagesBySession[DEFAULT_SESSION_ID] ?? demoMessages)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [panelOpen, setPanelOpen] = useState(false)
  const [activeTab, setActiveTab] = useState<ResultTab>('curves')
  const [draft, setDraft] = useState('')
  const [attachments, setAttachments] = useState<PendingAttachment[]>([])
  const [toast, setToast] = useState<string | null>(null)

  const isNarrow = useMediaQuery('(max-width: 1279px)')

  const toastTimer = useRef<number | undefined>(undefined)
  const fileIndex = useRef(0)

  const notify = useCallback((text: string) => {
    setToast(text)
    if (toastTimer.current) window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(null), 2200)
  }, [])

  const selectSession = (id: string) => {
    setSelectedId(id)
    setMessages(messagesBySession[id] ?? [])
    setPanelOpen(false)
  }

  const newSession = () => {
    const s: Session = {
      id: `s-${Date.now()}`,
      title: '新的仿真',
      status: 'running',
      updatedAt: new Date().toISOString(),
    }
    setSessions((prev) => [s, ...prev])
    setSelectedId(s.id)
    setMessages([])
    setPanelOpen(false)
    setActiveTab('curves')
  }

  const renameSession = (id: string, title: string) => {
    setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, title } : s)))
  }

  const deleteSession = (id: string) => {
    const remaining = sessions.filter((s) => s.id !== id)
    setSessions(remaining)
    if (selectedId === id) {
      const next = remaining[0] ?? null
      setSelectedId(next?.id ?? null)
      setMessages(next ? (messagesBySession[next.id] ?? []) : [])
      setPanelOpen(false)
    }
  }

  const send = () => {
    const text = draft.trim()
    if (!text && attachments.length === 0) return
    const ts = new Date().toISOString()
    const out: Message[] = []
    if (text) out.push({ id: `u-${Date.now()}`, type: 'user', timestamp: ts, content: text })
    for (const a of attachments) {
      out.push({
        id: `a-${Date.now()}-${a.id}`,
        type: 'attachment',
        timestamp: ts,
        file: { fileName: a.fileName, fileSize: a.fileSize, fileType: a.fileType },
      })
    }
    setMessages((prev) => [...prev, ...out])
    setDraft('')
    setAttachments([])
    // 真实后端接入点：在此把 text + attachments 提交给后端，并订阅 SSE 流式返回消息。
  }

  const addAttachment = () => {
    const f = MOCK_FILES[fileIndex.current % MOCK_FILES.length]
    fileIndex.current += 1
    setAttachments((prev) => [...prev, { ...f, id: `att-${Date.now()}-${prev.length}` }])
  }

  const handlers: MessageHandlers = {
    onViewCurves: () => {
      setPanelOpen(true)
      setActiveTab('curves')
    },
    onDownloadReport: (taskId) => notify(`下载报告 ${taskId}（mock）`),
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
          attachments={attachments}
          onAddAttachment={addAttachment}
          onRemoveAttachment={(id) => setAttachments((prev) => prev.filter((a) => a.id !== id))}
        />
      </main>

      <ResultPanel
        open={panelOpen}
        activeTab={activeTab}
        overlay={isNarrow}
        onTabChange={setActiveTab}
        onClose={() => setPanelOpen(false)}
        voltageCurve={voltageCurve}
        powerCurve={powerCurve}
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
