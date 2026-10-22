import type { ExampleDetail as Detail } from '../../types'
export default function ExampleDetail({example}:{example:Detail}){
  return <div aria-label={`案例详情“${example.title}”`}><h3>{example.title}</h3><p>{example.entryPath}</p><p>{example.files.length} 个文件</p><p>快照树哈希：{example.treeHash}</p></div>
}
