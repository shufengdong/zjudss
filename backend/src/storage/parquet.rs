use std::path::Path;

use arrow_array::RecordBatch;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetColumn {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParquetMetadata {
    pub columns: Vec<ParquetColumn>,
    pub row_count: u64,
    pub size_bytes: u64,
    pub snappy: bool,
}

#[derive(Debug)]
pub struct ParquetError {
    code: &'static str,
    message: String,
}
impl ParquetError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for ParquetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ParquetError {}

pub struct ParquetInspector;
impl ParquetInspector {
    pub fn metadata(path: &Path) -> Result<ParquetMetadata, ParquetError> {
        let file = std::fs::File::open(path)
            .map_err(|e| ParquetError::new("parquet_read_failed", e.to_string()))?;
        let size_bytes = file
            .metadata()
            .map_err(|e| ParquetError::new("parquet_read_failed", e.to_string()))?
            .len();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| ParquetError::new("parquet_footer_invalid", e.to_string()))?;
        let columns = builder
            .schema()
            .fields()
            .iter()
            .map(|field| ParquetColumn {
                name: field.name().clone(),
                dtype: field.data_type().to_string(),
                nullable: field.is_nullable(),
                unit: field.metadata().get("unit").cloned(),
            })
            .collect();
        let row_count = u64::try_from(builder.metadata().file_metadata().num_rows())
            .map_err(|_| ParquetError::new("parquet_metadata_invalid", "负数 row count"))?;
        let mut chunks = builder
            .metadata()
            .row_groups()
            .iter()
            .flat_map(|group| group.columns());
        let first = chunks.next();
        let snappy = match first {
            Some(column) => {
                column.compression() == Compression::SNAPPY
                    && chunks.all(|column| column.compression() == Compression::SNAPPY)
            }
            None => row_count == 0,
        };
        Ok(ParquetMetadata {
            columns,
            row_count,
            size_bytes,
            snappy,
        })
    }

    pub fn page(
        path: &Path,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, ParquetError> {
        Self::projection(path, &[], offset, limit)
    }

    pub fn projection(
        path: &Path,
        columns: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, ParquetError> {
        if limit == 0 || limit > 100_000 {
            return Err(ParquetError::new(
                "parquet_page_invalid",
                "limit 必须在 1..=100000",
            ));
        }
        let file = std::fs::File::open(path)
            .map_err(|e| ParquetError::new("parquet_read_failed", e.to_string()))?;
        let mut builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| ParquetError::new("parquet_footer_invalid", e.to_string()))?;
        if !columns.is_empty() {
            for column in columns {
                if builder.schema().index_of(column).is_err() {
                    return Err(ParquetError::new("parquet_column_not_found", column));
                }
            }
            let mask = ProjectionMask::columns(
                builder.parquet_schema(),
                columns.iter().map(String::as_str),
            );
            builder = builder.with_projection(mask);
        }
        builder
            .with_offset(offset)
            .with_limit(limit)
            .build()
            .map_err(|e| ParquetError::new("parquet_read_failed", e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ParquetError::new("parquet_read_failed", e.to_string()))
    }
}
