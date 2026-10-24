import{fireEvent,render,screen}from'@testing-library/react'
import{describe,expect,it}from'vitest'
import type{ToolCallMessage}from'../../types'
import ToolCallCard from'./ToolCallCard'

const message:ToolCallMessage={
  id:'1',type:'tool-call',timestamp:'2026-10-24T00:00:00Z',callId:'call-1',
  toolName:'search_engine_docs',args:{query:'潮流计算函数',max_chars:3000},status:'success',
  summary:'这里是一段很长的文档检索结果',
}

describe('ToolCallCard',()=>{
  it('keeps tool details collapsed until the user expands them',()=>{
    render(<ToolCallCard message={message}/>)
    expect(screen.getByText('search_engine_docs')).toBeInTheDocument()
    expect(screen.getByText('成功')).toBeInTheDocument()
    expect(screen.queryByText(/query:/)).not.toBeInTheDocument()
    expect(screen.queryByText(message.summary!)).not.toBeInTheDocument()

    const toggle=screen.getByRole('button',{name:'查看详情'})
    expect(toggle).toHaveAttribute('aria-expanded','false')
    fireEvent.click(toggle)
    expect(screen.getByText(/query:/)).toBeInTheDocument()
    expect(screen.getByText(message.summary!)).toBeInTheDocument()
    expect(screen.getByRole('button',{name:'收起'})).toHaveAttribute('aria-expanded','true')

    fireEvent.click(screen.getByRole('button',{name:'收起'}))
    expect(screen.queryByText(/query:/)).not.toBeInTheDocument()
    expect(screen.queryByText(message.summary!)).not.toBeInTheDocument()
  })
})
