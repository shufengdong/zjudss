import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import Sidebar from './Sidebar'

const session = {
  id: '42',
  title: '馈线短路分析',
  status: 'completed' as const,
  updatedAt: '2026-10-24T08:00:00Z',
}

const preview = {
  impactToken: 'impact-token', modelCount: 2, dataResourceCount: 3, scriptCount: 4,
  runCount: 5, resultCount: 6, analysisArtifactCount: 7, logicalBytes: 8192,
  expiresAt: '2026-10-24T08:05:00Z',
}

function renderSidebar(onDelete = vi.fn().mockResolvedValue(undefined), onPreviewDelete = vi.fn().mockResolvedValue(preview)) {
  const onSelect = vi.fn()
  render(
    <Sidebar
      sessions={[session]}
      selectedId={session.id}
      collapsed={false}
      onSelect={onSelect}
      onNew={vi.fn()}
      onRename={vi.fn()}
      onDelete={onDelete}
      onPreviewDelete={onPreviewDelete}
      onToggleCollapse={vi.fn()}
    />,
  )
  return { onDelete, onPreviewDelete, onSelect }
}

function openDeleteConfirmation() {
  fireEvent.click(screen.getByRole('button', { name: '打开“馈线短路分析”的会话操作' }))
  fireEvent.click(screen.getByRole('button', { name: '删除“馈线短路分析”' }))
}

describe('Sidebar session deletion', () => {
  it('selects a session from the whole content area without selecting text', () => {
    const { onSelect } = renderSidebar()

    const sessionButton = screen.getByRole('button', { name: /打开会话.*馈线短路分析/ })
    expect(sessionButton).toContainElement(screen.getByText('已完成'))
    expect(sessionButton).toHaveClass('cursor-pointer', 'select-none')
    fireEvent.click(screen.getByText('已完成'))

    expect(onSelect).toHaveBeenCalledOnce()
    expect(onSelect).toHaveBeenCalledWith(session.id)
  })

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
        onPreviewDelete={vi.fn().mockResolvedValue(preview)}
        onToggleCollapse={vi.fn()}
      />,
    )

    expect(screen.getByRole('button', { name: /打开会话.*新会话/ })).toHaveTextContent('新会话')
  })

  it('loads an impact preview before confirmation and deletes only with its token', async () => {
    const { onDelete, onPreviewDelete } = renderSidebar()

    openDeleteConfirmation()

    expect(onDelete).not.toHaveBeenCalled()
    expect(onPreviewDelete).toHaveBeenCalledWith(session.id)
    expect(await screen.findByRole('dialog', { name: '删除会话' })).toBeInTheDocument()
    expect(screen.getByText(/2 个模型版本/)).toBeInTheDocument()
    expect(screen.getByText(/3 个数据资源/)).toBeInTheDocument()
    expect(screen.getByText(/4 个脚本/)).toBeInTheDocument()
    expect(screen.getByText(/5 条运行记录/)).toBeInTheDocument()
    expect(screen.getByText(/6 个结果数据集/)).toBeInTheDocument()
    expect(screen.getByText(/8 KB/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.queryByRole('dialog', { name: '删除会话' })).not.toBeInTheDocument()
    expect(onDelete).not.toHaveBeenCalled()

    openDeleteConfirmation()
    fireEvent.click(await screen.findByRole('button', { name: '确认删除' }))
    await waitFor(()=>expect(onDelete).toHaveBeenCalledTimes(1))
    expect(onDelete).toHaveBeenCalledWith(session.id, preview.impactToken)
  })

  it('closes the confirmation with Escape without deleting', async () => {
    const { onDelete } = renderSidebar()
    openDeleteConfirmation()
    await screen.findByRole('dialog', { name: '删除会话' })

    fireEvent.keyDown(document, { key: 'Escape' })

    expect(screen.queryByRole('dialog', { name: '删除会话' })).not.toBeInTheDocument()
    expect(onDelete).not.toHaveBeenCalled()
  })

  it('keeps the confirmation open and refreshes impact after deletion fails', async () => {
    const refreshed = { ...preview, impactToken: 'refreshed-token', runCount: 7 }
    const onDelete = vi.fn().mockRejectedValue(new Error('删除影响已变化'))
    const onPreviewDelete = vi.fn().mockResolvedValueOnce(preview).mockResolvedValueOnce(refreshed)
    renderSidebar(onDelete, onPreviewDelete)
    openDeleteConfirmation()

    fireEvent.click(await screen.findByRole('button', { name: '确认删除' }))

    expect(await screen.findByText('删除影响已变化')).toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: '删除会话' })).toBeInTheDocument()
    await waitFor(() => expect(onPreviewDelete).toHaveBeenCalledTimes(2))
    expect(screen.getByText(/7 条运行记录/)).toBeInTheDocument()
  })
})
