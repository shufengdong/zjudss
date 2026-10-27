import { useEffect, useMemo, useState, type ReactNode } from 'react'
import type { Session, SessionDeletionPreview, SessionStatus } from '../types'
import { MoreIcon, PanelLeftIcon, PencilIcon, PlusIcon, TrashIcon } from './icons'

interface SidebarProps {
  sessions: Session[]
  selectedId: string | null
  collapsed: boolean
  onSelect: (id: string) => void
  onNew: () => void
  onRename: (id: string, title: string) => void
  onPreviewDelete: (id: string) => Promise<SessionDeletionPreview>
  onDelete: (id: string, impactToken: string) => Promise<void>
  onToggleCollapse: () => void
  footer?: ReactNode
  search?: ReactNode
}

type Group = '今天' | '昨天' | '更早'

function groupOf(iso: string): Group {
  const d = new Date(iso).getTime()
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  if (d >= today) return '今天'
  if (d >= today - 24 * 60 * 60 * 1000) return '昨天'
  return '更早'
}

function formatTime(iso: string, group: Group): string {
  const d = new Date(iso)
  if (group === '更早') return `${d.getMonth() + 1}/${d.getDate()}`
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

const STATUS_CONFIG: Record<SessionStatus, { label: string; cls: string; dot: string }> = {
  completed: { label: '已完成', cls: 'bg-emerald-50 text-emerald-600', dot: 'bg-emerald-500' },
  running: { label: '运行中', cls: 'bg-brand-50 text-brand-600', dot: 'bg-brand-500' },
  failed: { label: '失败', cls: 'bg-red-50 text-red-600', dot: 'bg-red-500' },
}

const sessionTitle = (session: Pick<Session, 'title'>) => session.title.trim() || '新会话'

export default function Sidebar({
  sessions,
  selectedId,
  collapsed,
  onSelect,
  onNew,
  onRename,
  onPreviewDelete,
  onDelete,
  onToggleCollapse,
  footer,
  search,
}: SidebarProps) {
  const [menuId, setMenuId] = useState<string | null>(null)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [pendingDelete, setPendingDelete] = useState<Session | null>(null)
  const [deletionPreview, setDeletionPreview] = useState<SessionDeletionPreview | null>(null)
  const [deletionError, setDeletionError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)

  useEffect(() => {
    if (!pendingDelete) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setPendingDelete(null)
    }
    document.addEventListener('keydown', closeOnEscape)
    return () => document.removeEventListener('keydown', closeOnEscape)
  }, [pendingDelete])

  const groups = useMemo(() => {
    const order: Group[] = ['今天', '昨天', '更早']
    const map = new Map<Group, Session[]>()
    for (const g of order) map.set(g, [])
    for (const s of sessions) map.get(groupOf(s.updatedAt))?.push(s)
    return order.map((g) => ({ label: g, items: map.get(g) ?? [] })).filter((g) => g.items.length > 0)
  }, [sessions])

  const startRename = (s: Session) => {
    setEditingId(s.id)
    setDraft(s.title)
    setMenuId(null)
  }

  const commitRename = (id: string) => {
    const t = draft.trim()
    if (t) onRename(id, t)
    setEditingId(null)
  }

  const loadDeletionPreview = async (session:Session) => {
    setPendingDelete(session);setDeletionPreview(null);setDeletionError(null)
    try { setDeletionPreview(await onPreviewDelete(session.id)) }
    catch(error) { setDeletionError(error instanceof Error?error.message:'无法读取删除影响') }
  }

  const confirmDelete = async () => {
    if(!pendingDelete||!deletionPreview||deleting)return
    setDeleting(true);setDeletionError(null)
    try { await onDelete(pendingDelete.id,deletionPreview.impactToken);setPendingDelete(null);setDeletionPreview(null) }
    catch(error) {
      setDeletionError(error instanceof Error?error.message:'删除失败')
      try { setDeletionPreview(await onPreviewDelete(pendingDelete.id)) } catch { setDeletionPreview(null) }
    } finally { setDeleting(false) }
  }

  return (
    <>
    <aside
      className={`${collapsed ? 'w-0' : 'w-[260px]'} h-full shrink-0 overflow-hidden border-r border-slate-200 bg-slate-100/70 transition-[width] duration-200`}
    >
      <div className="flex h-full w-[260px] flex-col">
        <div className="flex items-center justify-between px-3 pb-1 pt-3">
          <button
            type="button"
            aria-label="新建仿真"
            onClick={onNew}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-brand-600"
          >
            <PlusIcon className="h-4 w-4" />
            新建仿真
          </button>
          <button
            type="button"
            aria-label="收起历史侧栏"
            onClick={onToggleCollapse}
            className="ml-2 rounded-md p-1.5 text-slate-400 hover:bg-slate-200 hover:text-slate-600"
            title="收起侧栏"
          >
            <PanelLeftIcon className="h-4 w-4" />
          </button>
        </div>

        {search && <div className="relative px-3 pb-2">{search}</div>}

        <div className="flex-1 overflow-y-auto px-2 py-2">
          {groups.map((g) => (
            <div key={g.label} className="mb-3">
              <div className="px-2 py-1 text-xs font-medium text-slate-400">{g.label}</div>
              <ul className="space-y-0.5">
                {g.items.map((s) => (
                  <li key={s.id} className="relative">
                    {editingId === s.id ? (
                      <input
                        aria-label={`重命名“${sessionTitle(s)}”`}
                        autoFocus
                        value={draft}
                        onChange={(e) => setDraft(e.target.value)}
                        onBlur={() => commitRename(s.id)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') commitRename(s.id)
                          if (e.key === 'Escape') setEditingId(null)
                        }}
                        className="w-full rounded-lg border border-brand-300 bg-white px-2 py-1.5 text-sm outline-none"
                      />
                    ) : (
                      <div
                        className={`group flex items-center rounded-lg transition-colors ${
                          selectedId === s.id ? 'bg-white shadow-sm' : 'hover:bg-slate-200/60'
                        }`}
                      >
                        <button
                          type="button"
                          onClick={() => onSelect(s.id)}
                          className="min-w-0 flex-1 cursor-pointer select-none px-2 py-2 text-left"
                        >
                          <span className="sr-only">打开会话</span>
                          <div className="truncate text-sm text-slate-700">{sessionTitle(s)}</div>
                          <div className="mt-0.5 flex items-center gap-1.5">
                            <span
                              className={`inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[11px] font-medium leading-none ${STATUS_CONFIG[s.status].cls}`}
                            >
                              <span
                                className={`h-1.5 w-1.5 rounded-full ${STATUS_CONFIG[s.status].dot} ${
                                  s.status === 'running' ? 'animate-pulse' : ''
                                }`}
                              />
                              {STATUS_CONFIG[s.status].label}
                            </span>
                            <span className="text-[11px] text-slate-400">{formatTime(s.updatedAt, groupOf(s.updatedAt))}</span>
                          </div>
                        </button>
                        <button
                          type="button"
                          aria-label={`打开“${sessionTitle(s)}”的会话操作`}
                          onClick={(e) => {
                            e.stopPropagation()
                            setMenuId(menuId === s.id ? null : s.id)
                          }}
                          className="mr-2 rounded-md p-1 text-slate-400 opacity-0 transition-opacity hover:bg-slate-200 hover:text-slate-600 group-hover:opacity-100"
                          title="更多操作"
                        >
                          <MoreIcon className="h-4 w-4" />
                        </button>
                      </div>
                    )}

                    {menuId === s.id && (
                      <>
                        <button
                          type="button"
                          className="fixed inset-0 z-10 cursor-default"
                          onClick={() => setMenuId(null)}
                          aria-label="关闭菜单"
                        />
                        <div className="absolute right-2 top-9 z-20 w-28 overflow-hidden rounded-lg border border-slate-200 bg-white py-1 shadow-lg">
                          <button
                            type="button"
                            aria-label={`重命名“${sessionTitle(s)}”`}
                            onClick={() => startRename(s)}
                            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-slate-600 hover:bg-slate-50"
                          >
                            <PencilIcon className="h-3.5 w-3.5" />
                            重命名
                          </button>
                          <button
                            type="button"
                            aria-label={`删除“${sessionTitle(s)}”`}
                            onClick={() => {
                              setMenuId(null)
                              void loadDeletionPreview(s)
                            }}
                            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-red-600 hover:bg-red-50"
                          >
                            <TrashIcon className="h-3.5 w-3.5" />
                            删除
                          </button>
                        </div>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        {footer}
      </div>
    </aside>
    {pendingDelete && (
      <div
        className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 p-4"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) setPendingDelete(null)
        }}
      >
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="delete-session-title"
          className="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl"
        >
          <h2 id="delete-session-title" className="text-base font-semibold text-slate-900">删除会话</h2>
          <p className="mt-3 text-sm leading-6 text-slate-600">
            确定删除“{sessionTitle(pendingDelete)}”吗？该操作无法恢复。
          </p>
          {!deletionPreview&&!deletionError&&<p className="mt-3 text-sm text-slate-500">正在计算删除影响…</p>}
          {deletionPreview&&<ul className="mt-3 space-y-1 text-sm text-slate-600">
            <li>{deletionPreview.modelCount} 个模型版本</li><li>{deletionPreview.dataResourceCount} 个数据资源</li>
            <li>{deletionPreview.scriptCount} 个脚本</li><li>{deletionPreview.runCount} 条运行记录</li>
            <li>{deletionPreview.resultCount} 个结果数据集</li><li>{deletionPreview.analysisArtifactCount} 个分析产物</li>
            <li>{Math.ceil(deletionPreview.logicalBytes/1024)} KB 逻辑空间</li>
          </ul>}
          {deletionError&&<p role="alert" className="mt-3 text-sm text-red-600">{deletionError}</p>}
          <div className="mt-6 flex justify-end gap-2">
            <button
              type="button"
              autoFocus
              disabled={deleting}
              onClick={() => setPendingDelete(null)}
              className="rounded-lg border border-slate-300 px-4 py-2 text-sm text-slate-700 hover:bg-slate-50"
            >
              取消
            </button>
            <button
              type="button"
              disabled={!deletionPreview||deleting}
              onClick={() => void confirmDelete()}
              className="rounded-lg bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700"
            >
              {deleting?'正在删除…':'确认删除'}
            </button>
          </div>
        </div>
      </div>
    )}
    </>
  )
}
