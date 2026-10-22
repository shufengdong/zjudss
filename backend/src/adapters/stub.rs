use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use super::{
    AdapterError, EngineRunOutput, InspectOutcome, InspectSpec, OutputLimits, RunMode, RunSpec,
    SimEngine, validate_outputs,
};

#[derive(Debug, Clone, Default)]
pub struct StubEngine;

impl StubEngine {
    fn run_fixture(
        &self,
        spec: RunSpec,
        allowed: &[RunMode],
    ) -> Result<EngineRunOutput, AdapterError> {
        spec.validate()?;
        if !allowed.contains(&spec.mode) {
            return Err(AdapterError::new(
                "run_mode_invalid",
                super::ErrorCategory::Model,
                "stub 入口与运行模式不匹配",
            ));
        }
        std::fs::create_dir_all(&spec.result_dir).map_err(|e| {
            AdapterError::new(
                "result_dir_create_failed",
                super::ErrorCategory::Io,
                e.to_string(),
            )
        })?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("value", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["stub"])),
                Arc::new(Int64Array::from(vec![1])),
            ],
        )
        .map_err(|e| {
            AdapterError::new(
                "stub_parquet_failed",
                super::ErrorCategory::Internal,
                e.to_string(),
            )
        })?;
        let file = std::fs::File::create(spec.result_dir.join("result.parquet")).map_err(|e| {
            AdapterError::new(
                "stub_parquet_failed",
                super::ErrorCategory::Io,
                e.to_string(),
            )
        })?;
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props)).map_err(|e| {
            AdapterError::new(
                "stub_parquet_failed",
                super::ErrorCategory::Internal,
                e.to_string(),
            )
        })?;
        writer
            .write(&batch)
            .and_then(|_| writer.close().map(|_| ()))
            .map_err(|e| {
                AdapterError::new(
                    "stub_parquet_failed",
                    super::ErrorCategory::Internal,
                    e.to_string(),
                )
            })?;
        validate_outputs(&spec.result_dir, OutputLimits::default())
    }
}

#[async_trait]
impl SimEngine for StubEngine {
    async fn inspect(&self, spec: InspectSpec) -> Result<InspectOutcome, AdapterError> {
        spec.validate()?;
        Ok(InspectOutcome {
            ok: true,
            summary: "model ok".into(),
            diagnostics: None,
        })
    }
    async fn run_dataframe(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.run_fixture(spec, &[RunMode::Rss, RunMode::Dataframe])
    }
    async fn run_bundle(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.run_fixture(spec, &[RunMode::Bundle])
    }
    async fn run_analysis(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.run_fixture(spec, &[RunMode::Analysis])
    }
}
