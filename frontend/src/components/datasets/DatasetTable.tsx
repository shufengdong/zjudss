import type{DatasetMetadata,DatasetPage}from'../../types'
const show=(value:unknown)=>value===null||value===undefined?'—':typeof value==='object'?JSON.stringify(value):String(value)
export default function DatasetTable({metadata,page,loading,onMore}:{metadata:DatasetMetadata;page:DatasetPage|null;loading:boolean;onMore:()=>void}){
  const visible=page?metadata.columns.filter(column=>page.columns.includes(column.name)):metadata.columns
  const columns=visible.map(c=>c.name),indices=columns.map(name=>page?.columns.indexOf(name)??-1)
  return <div className="overflow-auto"><table className="min-w-full border-collapse text-xs"><thead className="sticky top-0 bg-slate-100">
    <tr>{visible.map(c=><th key={c.name} className="whitespace-nowrap border-b px-3 py-2 text-left">{c.name}{c.unit?<span className="text-slate-400"> ({c.unit})</span>:null}</th>)}</tr>
  </thead><tbody>{page?.rows.map((row,i)=><tr key={i} className="odd:bg-white even:bg-slate-50">{indices.map((index,j)=><td key={columns[j]} className="max-w-72 truncate border-b px-3 py-2">{show(index<0?null:row[index])}</td>)}</tr>)}</tbody></table>
  {page?.nextCursor&&<button disabled={loading} onClick={onMore} className="m-3 rounded-lg border px-3 py-1.5 text-xs">{loading?'加载中…':'加载更多'}</button>}</div>
}
