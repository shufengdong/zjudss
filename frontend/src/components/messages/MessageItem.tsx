import type { Message, MessageHandlers } from '../../types'
import UserMessageCard from './UserMessage'
import AssistantMessageCard from './AssistantMessage'
import ToolCallCard from './ToolCallCard'
import ProgressCard from './ProgressCard'
import ResultCard from './ResultCard'
import AttachmentCard from './AttachmentCard'
import ComparisonCard from './ComparisonCard'
import ModelStatusCard from './ModelStatusCard'

interface MessageItemProps {
  message: Message
  handlers: MessageHandlers
}

export default function MessageItem({ message, handlers }: MessageItemProps) {
  switch (message.type) {
    case 'user':
      return <UserMessageCard message={message} />
    case 'assistant':
      return <AssistantMessageCard message={message} />
    case 'tool-call':
      return <ToolCallCard message={message} />
    case 'progress':
      return <ProgressCard message={message} />
    case 'result':
      return <ResultCard message={message} handlers={handlers} />
    case 'attachment':
      return (
        <AttachmentCard
          message={message}
          onDownload={(file) => handlers.notify(`原始上传文件由模型版本管理：${file.fileName}`)}
        />
      )
    case 'model-status':
      return <ModelStatusCard message={message} />
    case 'comparison':
      return <ComparisonCard message={message} handlers={handlers} />
    default:
      return null
  }
}
