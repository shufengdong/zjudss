import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import LoginPage from './LoginPage'

describe('LoginPage', () => {
  it('submits with Enter and prevents duplicate login attempts', async () => {
    let finish!: (value: { user: { id: string; name: string } }) => void
    const authenticate = vi.fn(() => new Promise<{ user: { id: string; name: string } }>((resolve) => {
      finish = resolve
    }))
    const onAuthenticated = vi.fn()
    render(<LoginPage authenticate={authenticate} onAuthenticated={onAuthenticated} />)

    const loginButton = screen.getByRole('button', { name: '登录' })
    fireEvent.change(screen.getByLabelText('用户 ID'), { target: { value: 'alice' } })
    fireEvent.change(screen.getByLabelText('密码'), { target: { value: 'alice' } })
    fireEvent.keyDown(screen.getByLabelText('密码'), { key: 'Enter' })
    expect(authenticate).toHaveBeenCalledTimes(1)
    fireEvent.click(loginButton)
    expect(authenticate).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: '登录中…' })).toBeDisabled()

    finish({ user: { id: 'alice', name: 'Alice' } })
    await waitFor(() => expect(onAuthenticated).toHaveBeenCalledWith({ id: 'alice', name: 'Alice' }))
  })

  it('shows one credential error without disclosing which field was wrong', async () => {
    const authenticate = vi.fn().mockRejectedValue(new Error('server detail'))
    render(<LoginPage authenticate={authenticate} onAuthenticated={() => undefined} />)
    fireEvent.change(screen.getByLabelText('用户 ID'), { target: { value: 'unknown' } })
    fireEvent.change(screen.getByLabelText('密码'), { target: { value: 'wrong' } })
    fireEvent.click(screen.getByRole('button', { name: '登录' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('用户 ID 或密码错误')
    expect(screen.queryByText('server detail')).not.toBeInTheDocument()
  })
})
