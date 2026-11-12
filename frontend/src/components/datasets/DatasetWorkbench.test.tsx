import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useChartView } from '../../hooks/useChartView'
import { useDataset } from '../../hooks/useDataset'
import DatasetWorkbench from './DatasetWorkbench'

vi.mock('../../hooks/useDataset', () => ({ useDataset: vi.fn() }))
vi.mock('../../hooks/useChartView', () => ({ useChartView: vi.fn() }))

describe('DatasetWorkbench', () => {
  it('downloads the protected Parquet directly without constructing a browser Blob', () => {
    vi.mocked(useDataset).mockReturnValue({
      metadata: {
        id: '20', runId: '36', analysisRunId: null, name: 'bus_voltage', rowCount: 26_370_610,
        sizeBytes: 552_121_377, columns: [{ name: 'bus', dtype: 'Utf8View', nullable: true }],
        views: [], schemaHash: 'schema-hash', lineage: [],
      },
      page: { datasetId: '20', columns: ['bus'], rows: [['source']], nextCursor: null, diagnostics: {} },
      loading: false,
      error: null,
      loadMore: vi.fn(async () => undefined),
      remove: vi.fn(async () => undefined),
      reload: vi.fn(),
    })
    vi.mocked(useChartView).mockReturnValue({ data: null, error: null, loading: false })

    render(<DatasetWorkbench datasetId="20" onClose={vi.fn()} />)

    const download = screen.getByRole('link', { name: '下载 Parquet' })
    expect(download).toHaveAttribute('href', '/api/datasets/20/download')
    expect(download).toHaveAttribute('download')
  })
})
