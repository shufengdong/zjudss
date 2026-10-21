import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

export default function Markdown({ children }: { children: string }) {
  return (
    <div className="prose prose-sm max-w-none prose-headings:text-slate-800 prose-p:text-slate-700 prose-li:text-slate-700 prose-a:text-brand-600 prose-strong:text-slate-800 prose-code:text-brand-700 prose-code:before:content-none prose-code:after:content-none prose-pre:bg-slate-900 prose-pre:text-slate-100 prose-th:text-slate-700 prose-td:text-slate-700">
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  )
}
