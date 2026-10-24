import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import Sidebar from './Sidebar'

const session = {
  id: '42',
  title: '馈线短路分析',
  status: 'completed' as const,
  updatedAt: '2026-10-24T08:00:00Z',
}

function renderSidebar(onDelete = vi.fn()) {
  render(
    <Sidebar
      sessions={[session]}
      selectedId={session.id}
      collapsed={false}
      onSelect={vi.fn()}
      onNew={vi.fn()}
      onRename={vi.fn()}
      onDelete={onDelete}
      onToggleCollapse={vi.fn()}
    />,
  )
  return onDelete
}

function openDeleteConfirmation() {
  fireEvent.click(screen.getByRole('button', { name: '打开“馈线短路分析”的会话操作' }))
  fireEvent.click(screen.getByRole('button', { name: '删除“馈线短路分析”' }))
}

describe('Sidebar session deletion', () => {
  it('shows a meaningful title for legacy sessions with an empty title', () => {
    render(
      <Sidebar
        sessions={[{ ...session, title: '' }]}
        selectedId={session.id}
        collapsed={false}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        onRename={vi.fn()}
        onDelete={vi.fn()}
        onToggleCollapse={vi.fn()}
      />,
    )

    expect(screen.getByRole('button', { name: '打开会话“新会话”' })).toHaveTextContent('新会话')
  })

  it('requires explicit confirmation and explains the irreversible impact', () => {
    const onDelete = renderSidebar()

    openDeleteConfirmation()

    expect(onDelete).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: '删除会话' })).toBeInTheDocument()
    expect(screen.getByText(/聊天记录、模型、脚本、仿真运行和结果/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.queryByRole('dialog', { name: '删除会话' })).not.toBeInTheDocument()
    expect(onDelete).not.toHaveBeenCalled()

    openDeleteConfirmation()
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }))
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onDelete).toHaveBeenCalledWith(session.id)
  })

  it('closes the confirmation with Escape without deleting', () => {
    const onDelete = renderSidebar()
    openDeleteConfirmation()

    fireEvent.keyDown(document, { key: 'Escape' })

    expect(screen.queryByRole('dialog', { name: '删除会话' })).not.toBeInTheDocument()
    expect(onDelete).not.toHaveBeenCalled()
  })
})
