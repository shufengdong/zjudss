import Markdown from '../Markdown'
import type { AssistantMessage } from '../../types'

export default function AssistantMessageCard({ message }: { message: AssistantMessage }) {
  return (
    <div className="flex gap-3 px-4 py-3 sm:px-6">
      <div className="flex h-8 w-8 shrink-0 select-none items-center justify-center rounded-full bg-brand-500 text-xs font-semibold text-white">
        AI
      </div>
      <div className="min-w-0 flex-1 pt-0.5">
        <Markdown>{message.content}</Markdown>
      </div>
    </div>
  )
}
