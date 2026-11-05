use std::path::PathBuf;

use crate::adapters::EngineDatasetOutput;
use crate::storage::{ParquetInspector, ParquetMetadata};

#[derive(Debug, Clone)]
pub(crate) struct InspectedExecutionOutput {
    pub name: String,
    pub path: PathBuf,
    pub metadata: ParquetMetadata,
}

#[derive(Debug)]
pub(crate) enum ExecutionResultError {
    FileTooLarge { name: String, size_bytes: u64 },
    Invalid(String),
}

impl std::fmt::Display for ExecutionResultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileTooLarge { name, size_bytes } => {
                write!(formatter, "结果 {name} 超过大小限制: {size_bytes}")
            }
            Self::Invalid(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ExecutionResultError {}

pub(crate) fn inspect_engine_outputs(
    outputs: &[EngineDatasetOutput],
    max_file_bytes: u64,
) -> Result<Vec<InspectedExecutionOutput>, ExecutionResultError> {
    outputs
        .iter()
        .map(|output| {
            let metadata = ParquetInspector::metadata(&output.path)
                .map_err(|error| ExecutionResultError::Invalid(error.to_string()))?;
            if metadata.size_bytes > max_file_bytes {
                return Err(ExecutionResultError::FileTooLarge {
                    name: output.name.clone(),
                    size_bytes: metadata.size_bytes,
                });
            }
            if !metadata.snappy {
                return Err(ExecutionResultError::Invalid(
                    "运行结果必须是 Snappy Parquet".into(),
                ));
            }
            Ok(InspectedExecutionOutput {
                name: output.name.clone(),
                path: output.path.clone(),
                metadata,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;

    use crate::adapters::EngineDatasetOutput;
    use crate::storage::ParquetInspector;

    use super::inspect_engine_outputs;

    #[test]
    fn empty_physical_output_set_is_left_to_business_policy() {
        assert!(inspect_engine_outputs(&[], 1).unwrap().is_empty());
    }

    #[test]
    fn zero_row_snappy_parquet_is_a_physical_fact_not_a_policy_rejection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.parquet");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            true,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(Vec::<Option<i64>>::new()))],
        )
        .unwrap();
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(
            std::fs::File::create(&path).unwrap(),
            schema,
            Some(properties),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let output = EngineDatasetOutput {
            name: "empty".into(),
            path: path.clone(),
            metadata: ParquetInspector::metadata(&path).unwrap(),
        };

        let inspected = inspect_engine_outputs(&[output], 1024 * 1024).unwrap();
        assert_eq!(inspected[0].metadata.row_count, 0);
    }
}
