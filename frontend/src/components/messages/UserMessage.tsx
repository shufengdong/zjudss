import type { UserMessage } from '../../types'

export default function UserMessageCard({ message }: { message: UserMessage }) {
  return (
    <div className="flex justify-end px-4 py-2 sm:px-6">
      <div className="max-w-[78%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-brand-500 px-4 py-2.5 text-sm leading-relaxed text-white shadow-sm">
        {message.content}
      </div>
    </div>
  )
}
