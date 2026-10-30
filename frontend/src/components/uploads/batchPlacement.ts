import type { UploadBatchView } from '../../types'

export type UploadBatchWorkbenchTab = 'workspace' | 'resources'

export const uploadBatchWorkbenchTab = (batch?: Pick<UploadBatchView, 'sourceKind'>): UploadBatchWorkbenchTab => batch?.sourceKind === 'case' ? 'workspace' : 'resources'

export const isClosedUploadBatch = (batch: Pick<UploadBatchView, 'status'>) => ['confirmed', 'discarded', 'expired'].includes(batch.status)
