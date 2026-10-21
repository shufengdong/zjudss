import type { Attachment, AttachmentMessage } from '../../types'
import { DocIcon, DownloadIcon, FileIcon, TableIcon } from '../icons'

function FileTypeIcon({ type }: { type: string }) {
  if (type === 'csv' || type === 'xlsx') return <TableIcon className="h-5 w-5 text-emerald-500" />
  if (type === 'pdf') return <DocIcon className="h-5 w-5 text-red-500" />
  if (type === 'case') return <FileIcon className="h-5 w-5 text-brand-500" />
  return <FileIcon className="h-5 w-5 text-slate-400" />
}

export default function AttachmentCard({
  message,
  onDownload,
}: {
  message: AttachmentMessage
  onDownload: (file: Attachment) => void
}) {
  const { file } = message
  return (
    <div className="mx-4 my-2 sm:mx-6">
      <div className="inline-flex max-w-full items-center gap-3 rounded-lg border border-slate-200 bg-white px-3 py-2 shadow-sm">
        <FileTypeIcon type={file.fileType} />
        <div className="min-w-0">
          <div className="truncate text-sm font-medium text-slate-700">{file.fileName}</div>
          <div className="text-xs text-slate-400">{file.fileSize}</div>
        </div>
        <button
          type="button"
          onClick={() => onDownload(file)}
          className="ml-1 text-slate-400 transition-colors hover:text-brand-600"
          title="下载"
        >
          <DownloadIcon className="h-4 w-4" />
        </button>
      </div>
    </div>
  )
}
