import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useModelDataFrame } from '../../hooks/useModelDataFrame'
import type { ModelDataFrameSource, Workspace } from '../../types'
import ModelDataFrameWorkbench from './ModelDataFrameWorkbench'

vi.mock('../../hooks/useModelDataFrame', () => ({ useModelDataFrame: vi.fn() }))

const source: ModelDataFrameSource = {
  kind: 'model-draft', dataframeId: 'f1', draftId: 'd1', revision: 2, tableName: 'bus',
}
const loaded = (conflict = false): ReturnType<typeof useModelDataFrame> => ({
  metadata: { id: 'f1', scope: 'draft', ownerId: 'd1', revision: 2, tableName: 'bus', rowCount: 2, sizeBytes: 5, columns: [{ name: 'id', dtype: 'Utf8', nullable: false }, { name: 'v', dtype: 'Float64', nullable: true }], schemaHash: 's', contentHash: 'c' },
  page: { resourceId: 'f1', columns: ['id', 'v'], rows: [['b1', 1]], nextCursor: 'next', diagnostics: {} },
  loading: false, error: conflict ? '草稿已变化' : null, conflict,
  loadMore: vi.fn(async () => undefined), download: vi.fn(async () => ({ blob: new Blob(), filename: 'bus.parquet' })), reload: vi.fn(async () => undefined),
})

describe('ModelDataFrameWorkbench', () => {
  beforeEach(() => vi.mocked(useModelDataFrame).mockReturnValue(loaded()))

  it('shows a read-only table workbench without result-only controls', () => {
    render(<ModelDataFrameWorkbench source={source} onClose={vi.fn()} onSourceChange={vi.fn()} reloadWorkspace={vi.fn()} notify={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: '可变草稿 · 修订 2 · bus' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '选择列' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '下载 Parquet' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '加载更多' })).toBeInTheDocument()
    expect(screen.queryByText('删除结果')).not.toBeInTheDocument()
    expect(screen.queryByText('数据表')).not.toBeInTheDocument()
    expect(screen.queryByText(/血缘|分析/)).not.toBeInTheDocument()
  })

  it('reloads the workspace and relocates the same draft table after conflict', async () => {
    vi.mocked(useModelDataFrame).mockReturnValue(loaded(true))
    const next: Workspace = { id: 'w', sessionId: 's', active: null, activeModelImports: [], draft: { id: 'd2', workspaceId: 'w', baseVersionId: null, revision: 3, diagnostics: {}, createdAt: 'x', updatedAt: 'x', tables: [{ id: 'f2', tableName: 'bus', artifactId: 'a', schema: [], rowCount: 1, contentHash: 'h' }] } }
    const change = vi.fn()
    render(<ModelDataFrameWorkbench source={source} onClose={vi.fn()} onSourceChange={change} reloadWorkspace={vi.fn().mockResolvedValue(next)} notify={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: '重新加载最新修订' }))
    await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'model-draft', dataframeId: 'f2', draftId: 'd2', revision: 3, tableName: 'bus' }))
  })

  it('closes with a notice when the conflicted draft table disappeared', async () => {
    vi.mocked(useModelDataFrame).mockReturnValue(loaded(true))
    const next: Workspace = { id: 'w', sessionId: 's', active: null, draft: null, activeModelImports: [] }
    const close = vi.fn(), notify = vi.fn()
    render(<ModelDataFrameWorkbench source={source} onClose={close} onSourceChange={vi.fn()} reloadWorkspace={vi.fn().mockResolvedValue(next)} notify={notify} />)
    fireEvent.click(screen.getByRole('button', { name: '重新加载最新修订' }))
    await waitFor(() => expect(close).toHaveBeenCalled())
    expect(notify).toHaveBeenCalledWith('草稿中的 bus 表已不存在')
  })
})
