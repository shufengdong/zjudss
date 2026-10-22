import type { ExampleDetail as Detail } from '../../types'
export default function ExampleDetail({example}:{example:Detail}){
  return <div><h3>{example.title}</h3><p>{example.entryPath}</p><p>{example.files.length} 个文件</p></div>
}
