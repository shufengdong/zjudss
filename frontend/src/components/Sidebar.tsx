import { useMemo, useState } from 'react'
import type { Session, SessionStatus } from '../types'
import { MoreIcon, PanelLeftIcon, PencilIcon, PlusIcon, TrashIcon } from './icons'

interface SidebarProps {
  sessions: Session[]
  selectedId: string | null
  collapsed: boolean
  onSelect: (id: string) => void
  onNew: () => void
  onRename: (id: string, title: string) => void
  onDelete: (id: string) => void
  onToggleCollapse: () => void
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

export default function Sidebar({
  sessions,
  selectedId,
  collapsed,
  onSelect,
  onNew,
  onRename,
  onDelete,
  onToggleCollapse,
}: SidebarProps) {
  const [menuId, setMenuId] = useState<string | null>(null)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')

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

  return (
    <aside
      className={`${collapsed ? 'w-0' : 'w-[260px]'} shrink-0 overflow-hidden border-r border-slate-200 bg-slate-100/70 transition-[width] duration-200`}
    >
      <div className="flex h-full w-[260px] flex-col">
        <div className="flex items-center justify-between px-3 pb-1 pt-3">
          <button
            type="button"
            onClick={onNew}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-brand-600"
          >
            <PlusIcon className="h-4 w-4" />
            新建仿真
          </button>
          <button
            type="button"
            onClick={onToggleCollapse}
            className="ml-2 rounded-md p-1.5 text-slate-400 hover:bg-slate-200 hover:text-slate-600"
            title="收起侧栏"
          >
            <PanelLeftIcon className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-2 py-2">
          {groups.map((g) => (
            <div key={g.label} className="mb-3">
              <div className="px-2 py-1 text-xs font-medium text-slate-400">{g.label}</div>
              <ul className="space-y-0.5">
                {g.items.map((s) => (
                  <li key={s.id} className="relative">
                    {editingId === s.id ? (
                      <input
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
                        onClick={() => onSelect(s.id)}
                        className={`group flex cursor-pointer items-center gap-2 rounded-lg px-2 py-2 transition-colors ${
                          selectedId === s.id ? 'bg-white shadow-sm' : 'hover:bg-slate-200/60'
                        }`}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-sm text-slate-700">{s.title}</div>
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
                        </div>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation()
                            setMenuId(menuId === s.id ? null : s.id)
                          }}
                          className="rounded-md p-1 text-slate-400 opacity-0 transition-opacity hover:bg-slate-200 hover:text-slate-600 group-hover:opacity-100"
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
                            onClick={() => startRename(s)}
                            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-slate-600 hover:bg-slate-50"
                          >
                            <PencilIcon className="h-3.5 w-3.5" />
                            重命名
                          </button>
                          <button
                            type="button"
                            onClick={() => {
                              setMenuId(null)
                              onDelete(s.id)
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
      </div>
    </aside>
  )
}
