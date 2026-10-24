import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getExample, listExamples } from '../../api/client'
import type { ExampleDetail, ExampleSummary, ExampleTreeNode } from '../../types'
import ExampleLibrary from './ExampleLibrary'

vi.mock('../../api/client', () => ({ listExamples: vi.fn(), getExample: vi.fn() }))

const valid: ExampleSummary = {
  id: 'ieee-123', title: 'Run_IEEE123Bus', description: 'execution_plan · 3 个文件',
  sourceKind: 'rssbundle', categoryPath: ['IEEETestCases', '123Bus'],
  bundlePath: 'IEEETestCases/123Bus/Run_IEEE123Bus.rssbundle', studyType: 'execution_plan',
  solver: 'radial', compatibility: 'opendss-native', results: ['voltage'],
  sourceEntry: 'dss/IEEE123Master.dss', fileCount: 3, sizeBytes: 1024,
  verification: 'verified', importable: true, diagnostics: [],
}
const invalid: ExampleSummary = {
  id: 'ieee-broken', title: 'Broken_IEEE123Bus', description: 'unknown · 1 个文件',
  sourceKind: 'rssbundle', categoryPath: ['IEEETestCases', '123Bus'],
  bundlePath: 'IEEETestCases/123Bus/Broken_IEEE123Bus.rssbundle', studyType: null,
  solver: null, compatibility: null, results: [], sourceEntry: null, fileCount: 1,
  sizeBytes: 12, verification: 'invalid', importable: false,
  diagnostics: ['manifest_invalid'],
}
const manifest = {
  schemaVersion: 1, artifact: 'mems.dss-rss-bundle', generator: 'eig-rs/1',
  psaContractVersion: '0.1.0', entry: 'main.rss',
  study: { kind: 'execution_plan', compatibility: 'opendss-native', solver: 'radial', results: ['voltage'] },
  source: { entry: 'dss/IEEE123Master.dss', root: 'dss', treeSha256: 'source-hash' },
  files: [
    { path: 'dss/IEEE123Master.dss', role: 'dss-entry', sha256: 'dss-hash' },
    { path: 'main.rss', role: 'main-rss', sha256: 'rss-hash' },
  ],
  inputs: [],
}
const validDetail: ExampleDetail = {
  ...valid, entryPath: 'main.rss', manifest, treeHash: 'case-hash',
  files: [{ path: 'manifest.json', role: 'manifest', sha256: 'manifest-hash' }, ...manifest.files],
}
const invalidDetail: ExampleDetail = {
  ...invalid, entryPath: null, manifest: null, treeHash: 'invalid-hash',
  files: [{ path: 'manifest.json', role: 'manifest', sha256: 'invalid-manifest' }],
}
const tree: ExampleTreeNode[] = [{
  type: 'directory', name: 'IEEETestCases', path: ['IEEETestCases'], children: [{
    type: 'directory', name: '123Bus', path: ['IEEETestCases', '123Bus'], children: [
      { type: 'example', example: invalid },
      { type: 'example', example: valid },
    ],
  }],
}]
const catalog = { engineResourceSnapshot: 'ers-test', tree }

describe('ExampleLibrary', () => {
  beforeEach(() => {
    vi.mocked(listExamples).mockReset().mockResolvedValue(catalog)
    vi.mocked(getExample).mockReset().mockImplementation(async id => id === valid.id ? validDetail : invalidDetail)
  })

  it('browses folders, loads details and applies only a valid leaf', async () => {
    const onUse = vi.fn()
    render(<ExampleLibrary onClose={vi.fn()} onUse={onUse} />)
    const top = await screen.findByRole('button', { name: 'IEEETestCases' })
    expect(top).toHaveAttribute('aria-expanded', 'true')
    const nested = screen.getByRole('button', { name: '123Bus' })
    expect(nested).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(nested)
    expect(nested).toHaveAttribute('aria-expanded', 'true')

    fireEvent.click(screen.getByRole('button', { name: valid.title }))
    expect(await screen.findByLabelText(`案例详情“${valid.title}”`)).toBeInTheDocument()
    expect(getExample).toHaveBeenCalledWith(valid.id)
    expect(screen.getByText(valid.bundlePath)).toBeInTheDocument()
    expect(screen.getByText('dss-entry：1')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: `使用案例“${valid.title}”` }))
    expect(onUse).toHaveBeenCalledWith(valid.id, valid.title, 'ers-test')

    fireEvent.click(screen.getByRole('button', { name: '返回案例目录' }))
    fireEvent.click(screen.getByRole('button', { name: invalid.title }))
    const detail = await screen.findByLabelText(`案例详情“${invalid.title}”`)
    expect(within(detail).getByText('manifest_invalid')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: `使用案例“${invalid.title}”` })).not.toBeInTheDocument()
  })

  it('searches remotely and expands every returned ancestor', async () => {
    render(<ExampleLibrary onClose={vi.fn()} onUse={vi.fn()} />)
    await screen.findByRole('button', { name: 'IEEETestCases' })
    fireEvent.change(screen.getByRole('textbox', { name: '搜索内置案例' }), { target: { value: 'voltage' } })
    await waitFor(() => expect(listExamples).toHaveBeenLastCalledWith({ query: 'voltage' }))
    expect(screen.getByRole('button', { name: 'IEEETestCases' })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('button', { name: '123Bus' })).toHaveAttribute('aria-expanded', 'true')
    expect(await screen.findByRole('button', { name: valid.title })).toBeInTheDocument()
  })

  it('shows empty state and exposes an explicit detail return action', async () => {
    vi.mocked(listExamples).mockResolvedValue({ engineResourceSnapshot: 'ers-test', tree: [] })
    const empty = render(<ExampleLibrary onClose={vi.fn()} onUse={vi.fn()} />)
    expect(await screen.findByText('没有匹配案例')).toBeInTheDocument()
    empty.unmount()

    vi.mocked(listExamples).mockResolvedValue(catalog)
    const { unmount } = render(<ExampleLibrary onClose={vi.fn()} onUse={vi.fn()} />)
    await screen.findByRole('button', { name: 'IEEETestCases' })
    fireEvent.click(screen.getByRole('button', { name: '123Bus' }))
    fireEvent.click(screen.getByRole('button', { name: valid.title }))
    expect(await screen.findByRole('button', { name: '返回案例目录' })).toBeInTheDocument()
    unmount()
  })

  it('shows a retryable catalog error', async () => {
    vi.mocked(listExamples).mockRejectedValueOnce(new Error('network down')).mockResolvedValue(catalog)
    render(<ExampleLibrary onClose={vi.fn()} onUse={vi.fn()} />)
    expect(await screen.findByText('案例库加载失败')).toBeInTheDocument()
    expect(screen.getByText('network down')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '重试加载案例' }))
    expect(await screen.findByRole('button', { name: 'IEEETestCases' })).toBeInTheDocument()
    expect(listExamples).toHaveBeenCalledTimes(2)
  })
})
