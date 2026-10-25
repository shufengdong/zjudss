import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ApiError,
  downloadModelDataFrame,
  getModelDataFrame,
  getModelDataFrameRows,
} from '../api/client'
import type { ModelDataFrameMetadata, ModelDataFramePage, ModelDataFrameSource } from '../types'

export function mergeModelDataFramePages(
  current: ModelDataFramePage | null,
  next: ModelDataFramePage,
): ModelDataFramePage {
  if (!current) return next
  if (current.resourceId !== next.resourceId) throw new Error('模型表分页资源不一致')
  return { ...next, rows: [...current.rows, ...next.rows] }
}

function sourceIdentity(source: ModelDataFrameSource | null) {
  if (!source) return ''
  return source.kind === 'model-version'
    ? `${source.kind}:${source.dataframeId}:${source.versionId}:${source.tableName}`
    : `${source.kind}:${source.dataframeId}:${source.draftId}:${source.revision}:${source.tableName}`
}

function message(value: unknown) {
  return value instanceof Error ? value.message : '模型表加载失败'
}

function isAbort(value: unknown) {
  return value instanceof DOMException && value.name === 'AbortError'
}

export function useModelDataFrame(
  source: ModelDataFrameSource | null,
  selectedColumns?: string[],
) {
  const [metadata, setMetadata] = useState<ModelDataFrameMetadata | null>(null)
  const [page, setPage] = useState<ModelDataFramePage | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [conflict, setConflict] = useState(false)
  const generation = useRef(0)
  const activeController = useRef<AbortController | null>(null)
  const loadMoreGeneration = useRef<number | null>(null)
  const sourceRef = useRef(source)
  sourceRef.current = source
  const identity = sourceIdentity(source)
  const projection = selectedColumns?.join(',') || undefined

  const reload = useCallback(async () => {
    const requestGeneration = ++generation.current
    activeController.current?.abort()
    const current = sourceRef.current
    setMetadata(null)
    setPage(null)
    setError(null)
    setConflict(false)
    loadMoreGeneration.current = null
    setLoading(Boolean(current))
    if (!current) return
    const controller = new AbortController()
    activeController.current = controller
    try {
      const rowQuery = { limit: 100, ...(projection ? { columns: projection } : {}) }
      const [nextMetadata, nextPage] = await Promise.all([
        getModelDataFrame(current, controller.signal),
        getModelDataFrameRows(current, rowQuery, controller.signal),
      ])
      if (generation.current !== requestGeneration) return
      setMetadata(nextMetadata)
      setPage(nextPage)
    } catch (value) {
      if (generation.current !== requestGeneration || isAbort(value)) return
      setError(message(value))
      setConflict(value instanceof ApiError && value.code === 'draft_revision_conflict')
    } finally {
      if (generation.current === requestGeneration) {
        setLoading(false)
        if (activeController.current === controller) activeController.current = null
      }
    }
  }, [identity, projection])

  useEffect(() => {
    void reload()
    return () => activeController.current?.abort()
  }, [reload])

  const loadMore = useCallback(async () => {
    const current = sourceRef.current
    const requestGeneration = generation.current
    if (!current || !page?.nextCursor || conflict || loadMoreGeneration.current === requestGeneration) return
    loadMoreGeneration.current = requestGeneration
    const controller = new AbortController()
    activeController.current = controller
    setLoading(true)
    setError(null)
    try {
      const next = await getModelDataFrameRows(current, {
        limit: 100,
        cursor: page.nextCursor,
        ...(projection ? { columns: projection } : {}),
      }, controller.signal)
      if (generation.current === requestGeneration) {
        setPage(value => mergeModelDataFramePages(value, next))
      }
    } catch (value) {
      if (generation.current === requestGeneration && !isAbort(value)) {
        setError(message(value))
        setConflict(value instanceof ApiError && value.code === 'draft_revision_conflict')
      }
    } finally {
      if (generation.current === requestGeneration) setLoading(false)
      if (loadMoreGeneration.current === requestGeneration) loadMoreGeneration.current = null
      if (activeController.current === controller) activeController.current = null
    }
  }, [conflict, page?.nextCursor, projection])

  const download = useCallback(async () => {
    const current = sourceRef.current
    if (!current) return null
    try {
      return await downloadModelDataFrame(current, new AbortController().signal)
    } catch (value) {
      setError(message(value))
      setConflict(value instanceof ApiError && value.code === 'draft_revision_conflict')
      throw value
    }
  }, [identity])

  return { metadata, page, loading, error, conflict, loadMore, download, reload }
}
