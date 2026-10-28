import{fireEvent,render,screen,waitFor}from'@testing-library/react'
import{beforeEach,describe,expect,it,vi}from'vitest'
import{getScript}from'../../api/client'
import type{ScriptSummary}from'../../types'
import ScriptCard from'./ScriptCard'
vi.mock('../../api/client',()=>({getScript:vi.fn()}))
const script:ScriptSummary={id:'7',sessionId:'1',name:'flow.rss',sourceKind:'upload',contentHash:'h',validationStatus:'valid',requiredAliases:[],outputs:['result'],diagnostics:{},preview:'line 1\nline 2',previewTruncated:true,canRun:true,disabledReason:null,createdAt:'2026-01-01T00:00:00Z'}
describe('ScriptCard',()=>{beforeEach(()=>{vi.clearAllMocks();Object.assign(navigator,{clipboard:{writeText:vi.fn().mockResolvedValue(undefined)}})})
  it('loads full content only when expanded and collapses again',async()=>{vi.mocked(getScript).mockResolvedValue({...script,content:'full\ncontent'});render(<ScriptCard script={script} onRun={vi.fn()} notify={vi.fn()}/>);expect(getScript).not.toHaveBeenCalled();fireEvent.click(screen.getByText('展开全文'));await screen.findByText(/full/);expect(getScript).toHaveBeenCalledWith('7');fireEvent.click(screen.getByText('收起'));expect(screen.getByText(/line 1/)).toBeInTheDocument()})
  it('copies the full source and submits exactly once per click',async()=>{vi.mocked(getScript).mockResolvedValue({...script,content:'full source'});const run=vi.fn().mockResolvedValue(undefined);render(<ScriptCard script={script} onRun={run} notify={vi.fn()}/>);fireEvent.click(screen.getByText('复制'));await waitFor(()=>expect(navigator.clipboard.writeText).toHaveBeenCalledWith('full source'));fireEvent.click(screen.getByText('运行'));await waitFor(()=>expect(run).toHaveBeenCalledTimes(1))})
  it('allows a resource-only script without an active model',()=>{render(<ScriptCard script={script} onRun={vi.fn()} notify={vi.fn()}/>);expect(screen.getByText('运行')).toBeEnabled()})
})
