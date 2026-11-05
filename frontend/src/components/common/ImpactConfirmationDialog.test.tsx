import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ComponentProps } from 'react'
import { expect, it, vi } from 'vitest'
import ImpactConfirmationDialog from './ImpactConfirmationDialog'

const renderDialog = (overrides: Partial<ComponentProps<typeof ImpactConfirmationDialog>> = {}) => {
  const onCancel = vi.fn()
  const onConfirm = vi.fn()
  render(<ImpactConfirmationDialog
    title="删除测试资源"
    busy={false}
    loading={false}
    error={null}
    onCancel={onCancel}
    onConfirm={onConfirm}
    {...overrides}
  ><p>将影响 3 个结果</p></ImpactConfirmationDialog>)
  return { onCancel, onConfirm }
}

it('focuses cancellation, traps focus, and closes with Escape or the backdrop', async () => {
  const { onCancel } = renderDialog()
  const cancel = screen.getByRole('button', { name: '取消' })
  const confirm = screen.getByRole('button', { name: '确认删除' })
  await waitFor(() => expect(cancel).toHaveFocus())

  confirm.focus()
  fireEvent.keyDown(document, { key: 'Tab' })
  expect(cancel).toHaveFocus()
  fireEvent.keyDown(document, { key: 'Escape' })
  expect(onCancel).toHaveBeenCalledTimes(1)

  fireEvent.mouseDown(screen.getByTestId('impact-dialog-backdrop'))
  expect(onCancel).toHaveBeenCalledTimes(2)
})

it('keeps a busy dialog open and disables every dismiss action', () => {
  const { onCancel, onConfirm } = renderDialog({ busy: true })
  fireEvent.keyDown(document, { key: 'Escape' })
  fireEvent.mouseDown(screen.getByTestId('impact-dialog-backdrop'))
  fireEvent.click(screen.getByRole('button', { name: '正在删除…' }))

  expect(onCancel).not.toHaveBeenCalled()
  expect(onConfirm).not.toHaveBeenCalled()
  expect(screen.getByRole('button', { name: '取消' })).toBeDisabled()
})

it('renders loading and retryable error states without enabling confirmation', () => {
  const { rerender } = render(<ImpactConfirmationDialog
    title="删除测试资源"
    busy={false}
    loading
    error={null}
    onCancel={vi.fn()}
    onConfirm={vi.fn()}
  />)
  expect(screen.getByText('正在计算删除影响…')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: '确认删除' })).toBeDisabled()

  rerender(<ImpactConfirmationDialog
    title="删除测试资源"
    busy={false}
    loading={false}
    error="影响已变化"
    confirmDisabled
    onCancel={vi.fn()}
    onConfirm={vi.fn()}
  />)
  expect(screen.getByRole('alert')).toHaveTextContent('影响已变化')
  expect(screen.getByRole('button', { name: '确认删除' })).toBeDisabled()
})
