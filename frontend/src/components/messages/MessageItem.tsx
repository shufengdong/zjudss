import type { Message, MessageHandlers } from '../../types'
import UserMessageCard from './UserMessage'
import AssistantMessageCard from './AssistantMessage'
import ToolCallCard from './ToolCallCard'
import ProgressCard from './ProgressCard'
import ResultCard from './ResultCard'
import AttachmentCard from './AttachmentCard'
import ComparisonCard from './ComparisonCard'
import ModelStatusCard from './ModelStatusCard'
import ScriptCard from './ScriptCard'

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
    case 'script':
      return <ScriptCard script={message.script} onRun={handlers.onRunScript} notify={handlers.notify}/>
    case 'upload-batch':
      return <div className="mx-4 my-2 rounded-lg border bg-white p-3 text-sm">上传批次 {message.card.batchId}：{message.card.state}</div>
    case 'study-config':
      return <div className="mx-4 my-2 rounded-lg border bg-white p-3 text-sm">研究配置：{message.card.state}</div>
    case 'workflow-progress':
      return <div className="mx-4 my-2 rounded-lg border bg-white p-3 text-sm">研究工作流：{message.card.state}{message.card.attempts.length>0&&<details className="mt-2"><summary>执行尝试（{message.card.attempts.length}）</summary><ul>{message.card.attempts.map((attempt,index)=><li key={index}>第 {attempt.executionIndex} 次执行 / 第 {attempt.repairIndex} 次修正：{attempt.state}</li>)}</ul></details>}</div>
    default:
      return null
  }
}
