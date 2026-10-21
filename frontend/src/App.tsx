import { useCallback, useEffect, useRef, useState } from 'react'
import { powerCurve, voltageCurve } from './mocks/curves'
import type { Attachment, Message, MessageHandlers, ResultTab, Session } from './types'
import { useMediaQuery } from './hooks/useMediaQuery'
import { createSession, listMessages, listSessions, postMessage } from './api/client'
import { subscribeEvents } from './api/events'
import Sidebar from './components/Sidebar'
import Composer, { type PendingAttachment } from './components/Composer'
import ResultPanel from './components/ResultPanel'
import MessageItem from './components/messages/MessageItem'
import { ChevronDownIcon, PanelLeftIcon, PanelRightIcon } from './components/icons'

// 附件上传端点尚未实现（阶段4），这里仍是 mock，仅本地展示。
const MOCK_FILES: Attachment[] = [
  { fileName: '城区配电网模型.case', fileSize: '2.4 MB', fileType: 'case' },
  { fileName: '负荷曲线.csv', fileSize: '184 KB', fileType: 'csv' },
  { fileName: '拓扑校验报告.pdf', fileSize: '1.1 MB', fileType: 'pdf' },
]

// 乐观消息的临时 id 前缀：发送时先本地展示，等后端经 SSE 回传真实 id 后替换。
const LOCAL_USER = 'local-user-'
const LOCAL_ATT = 'local-att-'

// 把一条新事件合并进消息列表：
//  - user 事件 → 替换第一条乐观 user（FIFO，与发送顺序一致）
//  - progress → 按 taskId 归并（替换而非追加，api-design §5.4）
//  - 其它 → 按 id 去重后追加
function appendEvent(list: Message[], msg: Message): Message[] {
  if (msg.type === 'user') {
    const idx = list.findIndex((m) => m.type === 'user' && m.id.startsWith(LOCAL_USER))
    if (idx >= 0) {
      const next = list.slice()
      next[idx] = msg
      return next
    }
  }
  if (msg.type === 'progress') {
    const rest = list.filter((m) => !(m.type === 'progress' && m.taskId === msg.taskId))
    return [...rest, msg]
  }
  if (list.some((m) => m.id === msg.id)) return list
  return [...list, msg]
}

function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [panelOpen, setPanelOpen] = useState(false)
  const [activeTab, setActiveTab] = useState<ResultTab>('curves')
  const [draft, setDraft] = useState('')
  const [attachments, setAttachments] = useState<PendingAttachment[]>([])
  const [toast, setToast] = useState<string | null>(null)

  const isNarrow = useMediaQuery('(max-width: 1279px)')

  const toastTimer = useRef<number | undefined>(undefined)
  const fileIndex = useRef(0)
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

  // 订阅某会话的 SSE 事件流。回放完成后再订阅，避免 api-design §6 的竞态。
  const openEvents = useCallback(
    (id: string) => {
      closeEvents()
      closeEventsRef.current = subscribeEvents(id, (msg) => {
        setMessages((prev) => appendEvent(prev, msg))
      })
    },
    [closeEvents],
  )

  const selectSession = useCallback(
    async (id: string) => {
      if (selectedIdRef.current === id) return
      selectedIdRef.current = id
      setSelectedId(id)
      setMessages([])
      setPanelOpen(false)
      try {
        const list = await listMessages(id)
        if (selectedIdRef.current !== id) return // 已切换走，丢弃过期回放
        setMessages(list)
      } catch (err) {
        if (selectedIdRef.current === id) {
          notify(err instanceof Error ? err.message : '加载消息失败')
        }
      }
      openEvents(id)
    },
    [notify, openEvents],
  )

  const newSession = useCallback(async () => {
    try {
      const s = await createSession('新的仿真')
      setSessions((prev) => [s, ...prev])
      selectedIdRef.current = s.id
      setSelectedId(s.id)
      setMessages([])
      setPanelOpen(false)
      setActiveTab('curves')
      openEvents(s.id)
    } catch (err) {
      notify(err instanceof Error ? err.message : '新建会话失败')
    }
  }, [notify, openEvents])

  const renameSession = (id: string, title: string) => {
    // 阶段4：后端暂无 PATCH /api/sessions/{id}，仅本地生效，刷新后恢复。
    setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, title } : s)))
  }

  const deleteSession = (id: string) => {
    // 阶段4：后端暂无 DELETE /api/sessions/{id}，仅本地生效，刷新后恢复。
    const remaining = sessions.filter((s) => s.id !== id)
    setSessions(remaining)
    if (selectedId === id) {
      const next = remaining[0] ?? null
      selectedIdRef.current = next?.id ?? null
      setSelectedId(next?.id ?? null)
      setMessages([])
      setPanelOpen(false)
      if (next) void selectSession(next.id)
    }
  }

  const send = () => {
    const text = draft.trim()
    if ((!text && attachments.length === 0) || !selectedId) return
    const ts = new Date().toISOString()
    const optimistic: Message[] = []
    if (text) {
      optimistic.push({ id: `${LOCAL_USER}${Date.now()}`, type: 'user', timestamp: ts, content: text })
    }
    for (const a of attachments) {
      optimistic.push({
        id: `${LOCAL_ATT}${Date.now()}-${a.id}`,
        type: 'attachment',
        timestamp: ts,
        file: { fileName: a.fileName, fileSize: a.fileSize, fileType: a.fileType },
      })
    }

    const prevDraft = draft
    const prevAttachments = attachments

    setMessages((prev) => [...prev, ...optimistic])
    setDraft('')
    setAttachments([])

    // 仅文本触发后端；附件上传端点未实现（阶段4），附件仅本地展示。
    if (!text) return
    void postMessage(selectedId, text).catch((err) => {
      notify(err instanceof Error ? err.message : '发送失败')
      setMessages((prev) =>
        prev.filter((m) => !m.id.startsWith(LOCAL_USER) && !m.id.startsWith(LOCAL_ATT)),
      )
      setDraft(prevDraft)
      setAttachments(prevAttachments)
    })
  }

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
