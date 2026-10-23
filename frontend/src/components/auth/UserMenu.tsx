import { useState } from 'react'
import type { User } from '../../types'

export default function UserMenu({ user, onLogout }: { user: User; onLogout: () => void }) {
  const [open, setOpen] = useState(false)
  const initial = [...user.name.trim()][0] ?? [...user.id][0] ?? '?'
  return <div className="relative border-t border-slate-200 p-2">
    {open && <div className="absolute bottom-full left-2 right-2 mb-1 rounded-lg border border-slate-200 bg-white p-2 shadow-xl">
      <div className="truncate px-2 py-1 text-xs text-slate-500">用户 ID：{user.id}</div>
      <button type="button" onClick={() => { setOpen(false); onLogout() }} className="mt-1 w-full rounded-md px-2 py-1.5 text-left text-sm text-red-600 hover:bg-red-50">退出登录</button>
    </div>}
    <button type="button" onClick={() => setOpen(value => !value)}
      className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left hover:bg-slate-200/70">
      <span className="flex h-8 w-8 items-center justify-center rounded-full bg-brand-500 text-sm font-semibold text-white">{initial}</span>
      <span className="min-w-0 flex-1 truncate text-sm font-medium text-slate-700">{user.name}</span>
      <span aria-hidden className="text-slate-400">···</span>
    </button>
  </div>
}
