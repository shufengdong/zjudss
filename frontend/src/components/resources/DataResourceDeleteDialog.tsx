import type { DataResourceDeletionPreview } from '../../types'
import ImpactConfirmationDialog from '../common/ImpactConfirmationDialog'

export default function DataResourceDeleteDialog({preview,error,busy,onCancel,onConfirm}:{preview:DataResourceDeletionPreview|null;error:string|null;busy:boolean;onCancel:()=>void;onConfirm:()=>void}){
  return <ImpactConfirmationDialog title="删除数据文件" loading={!preview} error={error} busy={busy} confirmDisabled={!preview} onCancel={onCancel} onConfirm={onConfirm}>
    {preview&&<><p className="text-sm">将删除“{preview.displayName}”（{preview.alias}）及其当前文件。</p>
      <ul className="mt-3 space-y-1 text-xs text-slate-600">
        <li>{preview.pendingConfigurationCount} 个未确认配置将受影响</li>
        <li>{preview.frozenReferenceCount} 个冻结引用保留历史 artifact</li>
        <li>{preview.historicalReferenceCount} 个历史引用保留血缘</li>
      </ul>
    </>}
  </ImpactConfirmationDialog>
}
