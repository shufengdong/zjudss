import { useState } from 'react'
import { login } from '../../api/client'
import type { User } from '../../types'

interface LoginPageProps {
  authenticate?: typeof login
  onAuthenticated: (user: User) => void
}

export default function LoginPage({ authenticate = login, onAuthenticated }: LoginPageProps) {
  const [id, setId] = useState('')
  const [password, setPassword] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async () => {
    if (submitting || !id.trim() || !password) return
    setSubmitting(true)
    setError(null)
    try {
      const result = await authenticate(id.trim(), password)
      onAuthenticated(result.user)
    } catch {
      setError('用户 ID 或密码错误')
    } finally {
      setSubmitting(false)
    }
  }

  return <main className="flex min-h-screen items-center justify-center bg-slate-50 px-4">
    <form className="w-full max-w-sm rounded-2xl border border-slate-200 bg-white p-7 shadow-xl" onSubmit={event => { event.preventDefault(); void submit() }}>
      <div className="flex items-center justify-center gap-2">
        <img
          src={`${import.meta.env.BASE_URL}images/biglogo2.png`}
          alt="SGOOL 实验室标识"
          width="56"
          height="56"
          className="h-12 w-12 shrink-0 rounded-full object-contain sm:h-14 sm:w-14"
        />
        <h1 className="whitespace-nowrap text-lg font-semibold text-slate-900 sm:text-xl">配电网仿真智能体</h1>
      </div>
      <p className="mt-3 text-center text-sm text-slate-500">使用分配给你的用户 ID 登录</p>
      <label className="mt-6 block text-sm font-medium text-slate-700">
        用户 ID
        <input aria-label="用户 ID" autoComplete="username" autoFocus value={id} onChange={event => setId(event.target.value)}
          className="mt-2 w-full rounded-lg border border-slate-300 px-3 py-2 outline-none focus:border-brand-500" />
      </label>
      <label className="mt-4 block text-sm font-medium text-slate-700">
        密码
        <input aria-label="密码" type="password" autoComplete="current-password" value={password} onChange={event => setPassword(event.target.value)}
          onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); void submit() } }}
          className="mt-2 w-full rounded-lg border border-slate-300 px-3 py-2 outline-none focus:border-brand-500" />
      </label>
      {error && <p role="alert" className="mt-3 text-sm text-red-600">{error}</p>}
      <button type="submit" disabled={submitting || !id.trim() || !password}
        className="mt-5 w-full rounded-lg bg-brand-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-brand-600 disabled:cursor-not-allowed disabled:opacity-50">
        {submitting ? '登录中…' : '登录'}
      </button>
    </form>
  </main>
}
