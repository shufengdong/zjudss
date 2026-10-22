import{render,screen}from'@testing-library/react';import{describe,expect,it,vi}from'vitest';import DatasetTable from'./DatasetTable'
import{mergePages}from'../../hooks/useDataset'
describe('DatasetTable',()=>{it('uses schema order and renders null explicitly',()=>{
  render(<DatasetTable metadata={{id:'d',taskId:'t',analysisRunId:null,name:'result',rowCount:1,sizeBytes:10,columns:[{name:'b',dtype:'Utf8',nullable:true},{name:'a',dtype:'Int64',nullable:false}],views:[]}}
    page={{datasetId:'d',columns:['a','b'],rows:[[7,null]],nextCursor:null,diagnostics:{}}} loading={false} onMore={vi.fn()}/>)
  const headers=screen.getAllByRole('columnheader').map(v=>v.textContent);expect(headers).toEqual(['b','a']);expect(screen.getByText('—')).toBeInTheDocument()
})
it('keeps legitimate duplicate rows across cursor pages',()=>{
  const first={datasetId:'d',columns:['a'],rows:[[1]],nextCursor:'next',diagnostics:{}}
  const second={datasetId:'d',columns:['a'],rows:[[1]],nextCursor:null,diagnostics:{}}
  expect(mergePages(first,second).rows).toEqual([[1],[1]])
})
it('renders only projected columns in their requested order',()=>{
  render(<DatasetTable metadata={{id:'d',taskId:'t',analysisRunId:null,name:'result',rowCount:1,sizeBytes:10,columns:[{name:'a',dtype:'Int64',nullable:false},{name:'b',dtype:'Utf8',nullable:false}],views:[]}}
    page={{datasetId:'d',columns:['b'],rows:[['shown']],nextCursor:null,diagnostics:{}}} loading={false} onMore={vi.fn()}/>)
  expect(screen.queryByRole('columnheader',{name:'a'})).not.toBeInTheDocument()
  expect(screen.getByRole('columnheader',{name:'b'})).toBeInTheDocument()
  expect(screen.getByText('shown')).toBeInTheDocument()
})})
