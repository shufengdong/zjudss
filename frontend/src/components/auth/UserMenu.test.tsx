import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import UserMenu from './UserMenu'

describe('UserMenu', () => {
  it('shows the user name, reveals the ID, and logs out explicitly', () => {
    const onLogout = vi.fn()
    render(<UserMenu user={{ id: 'alice', name: 'Alice Zhang' }} onLogout={onLogout} />)
    expect(screen.getByRole('button', { name: '打开用户菜单' })).toHaveTextContent('Alice Zhang')
    expect(screen.queryByText('用户 ID：alice')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '打开用户菜单' }))
    expect(screen.getByText('用户 ID：alice')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '退出登录' }))
    expect(onLogout).toHaveBeenCalledTimes(1)
  })
})
