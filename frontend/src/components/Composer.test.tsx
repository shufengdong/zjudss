import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import Composer from './Composer'

describe('Composer script editor', () => {
  it('opens a large vertically resizable script editor', () => {
    render(
      <Composer
        value=""
        onChange={vi.fn()}
        onSend={vi.fn()}
        uploading={false}
        onUploadFiles={vi.fn()}
        onCreateScript={vi.fn()}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: '新建脚本' }))

    const dialog = screen.getByRole('dialog', { name: '新建脚本' })
    const editor = screen.getByRole('textbox', { name: '脚本内容' })
    expect(dialog).toHaveClass('max-w-4xl', 'max-h-[calc(100dvh-2rem)]', 'overflow-y-auto')
    expect(editor).toHaveClass(
      'h-[50vh]',
      'min-h-64',
      'sm:min-h-80',
      'max-h-[calc(100dvh-14rem)]',
      'resize-y',
      'overflow-auto',
    )
    expect(editor).toHaveAttribute('rows', '18')
    expect(editor).toHaveAccessibleDescription('可拖动编辑区右下角调整高度。')
  })
})
