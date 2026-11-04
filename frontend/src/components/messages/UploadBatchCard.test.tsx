import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { uploadBatch } from '../../test/fixtures'
import type { UploadBatchMessage, UploadBatchStatus } from '../../types'
import UploadBatchCard from './UploadBatchCard'

const displayBatch = (sourceKind: string, status: UploadBatchStatus = 'ready_to_confirm') => uploadBatch({
  sourceKind,
  status,
  diagnostics: { errors: [{ code: 'upload_warning', message: '需要检查上传内容' }] },
  reservedBytes: 100,
  expiresAtMs: 1,
  revision: 2,
  candidates: [
    { id: 'visible', displayName: 'load.csv', sheetName: 'load', proposedAlias: null, purpose: 'data_resource', evidence: {}, selected: true },
    { id: 'hidden', displayName: '隐藏说明', sheetName: 'help', proposedAlias: 'help', purpose: 'data_resource', evidence: { hidden: true }, selected: false },
  ],
})
const message: UploadBatchMessage = {
  id: 'message', type: 'upload-batch', timestamp: '2026-01-01T00:00:00Z', revision: 2,
  card: { batchId: '12', batchRevision: 2, state: 'ready_to_confirm', candidates: [{ id: 'visible', name: 'load.csv', state: 'ready' }], modelTransactionState: 'ready', resourceTransactionState: 'ready' },
}

describe('UploadBatchCard', () => {
  it('summarizes a live ordinary upload without duplicating its confirmation controls', () => {
    const onOpen = vi.fn()
    render(<UploadBatchCard message={message} batch={displayBatch('upload')} onOpen={onOpen} />)
    expect(screen.getByText(/2 个候选/)).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('需要检查上传内容')
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '确认用途并发布' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '前往数据文件' }))
    expect(onOpen).toHaveBeenCalledWith('12')
  })

  it('shows the conflicting resource aliases and retry guidance', () => {
    const failed = { ...displayBatch('case'), status: 'partially_confirmed' as const,
      resourcePublishStatus: 'failed' as const,
      diagnostics: { errors: [{ code: 'resource_alias_conflict', message: '以下数据资源与当前会话已有资源重名：solution（别名：solution）、para（别名：para）。请修改别名后重试数据发布。' }] } }
    render(<UploadBatchCard message={message} batch={failed} onOpen={vi.fn()} />)
    expect(screen.getByRole('alert')).toHaveTextContent('solution（别名：solution）')
    expect(screen.getByRole('alert')).toHaveTextContent('para（别名：para）')
    expect(screen.getByRole('alert')).toHaveTextContent('请修改别名后重试数据发布')
  })

  it('routes a live case summary to the model workspace', () => {
    const onOpen = vi.fn()
    render(<UploadBatchCard message={message} batch={displayBatch('case')} onOpen={onOpen} />)
    fireEvent.click(screen.getByRole('button', { name: '前往模型工作区' }))
    expect(onOpen).toHaveBeenCalledWith('12')
  })

  it('keeps a generic locator while the live batch is being restored', () => {
    const onOpen = vi.fn()
    render(<UploadBatchCard message={{ ...message, card: { ...message.card, diagnostic: { code: 'waiting', message: '正在恢复批次' } } }} onOpen={onOpen} />)
    expect(screen.getByText(/1 个候选/)).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('正在恢复批次')
    fireEvent.click(screen.getByRole('button', { name: '查看并确认' }))
    expect(onOpen).toHaveBeenCalledWith('12')
  })

  it('keeps closed history without offering a stale action', () => {
    render(<UploadBatchCard message={message} batch={displayBatch('case', 'confirmed')} onOpen={vi.fn()} />)
    expect(screen.getByText('confirmed')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })
})
