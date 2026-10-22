use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use serde::Serialize;

use super::{AdapterError, ErrorCategory};

pub const ENGINE_CONTRACT_VERSION: &str = "tensoreval-parquet-v2";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    pub contract_version: String,
    pub result_dir: bool,
    pub output_format_parquet: bool,
    pub named_parquet_inputs: bool,
}

impl EngineCapabilities {
    pub fn probe(binary: &Path) -> Result<Self, AdapterError> {
        let output = Command::new(binary)
            .arg("--help")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                AdapterError::new(
                    "engine_probe_failed",
                    ErrorCategory::Io,
                    format!("无法探测 tensoreval: {error}"),
                )
            })?;
        if !output.status.success() {
            return Err(AdapterError::new(
                "engine_probe_failed",
                ErrorCategory::Io,
                "tensoreval --help 执行失败",
            ));
        }
        let help = String::from_utf8_lossy(&output.stdout);
        let result_dir = help.contains("--result-dir");
        let output_format_parquet =
            help.contains("--output-format") && help.to_ascii_lowercase().contains("parquet");
        let declares_named_inputs = help.contains("--input")
            && (help.contains("NAME=FILE") || help.contains("<NAME=FILE>"));
        let named_parquet_inputs =
            declares_named_inputs && result_dir && output_format_parquet && parquet_smoke(binary);
        Ok(Self {
            contract_version: ENGINE_CONTRACT_VERSION.into(),
            result_dir,
            output_format_parquet,
            named_parquet_inputs,
        })
    }

    pub fn require_v2(&self) -> Result<(), AdapterError> {
        if self.contract_version != ENGINE_CONTRACT_VERSION {
            return Err(AdapterError::new(
                "engine_contract_mismatch",
                ErrorCategory::Internal,
                "tensoreval 合同版本不匹配",
            ));
        }
        if !self.result_dir {
            return Err(AdapterError::new(
                "engine_result_dir_unsupported",
                ErrorCategory::Internal,
                "tensoreval 缺少 --result-dir",
            ));
        }
        if !self.output_format_parquet {
            return Err(AdapterError::new(
                "engine_parquet_output_unsupported",
                ErrorCategory::Internal,
                "tensoreval 缺少 --output-format parquet",
            ));
        }
        if !self.named_parquet_inputs {
            return Err(AdapterError::new(
                "engine_parquet_input_unsupported",
                ErrorCategory::Internal,
                "tensoreval 缺少命名 Parquet input",
            ));
        }
        Ok(())
    }
}

fn parquet_smoke(binary: &Path) -> bool {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let root = std::env::temp_dir().join(format!(
        "zjudss-engine-probe-{}-{nonce}",
        std::process::id()
    ));
    let result = (|| -> Result<bool, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&root)?;
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(StringArray::from(vec!["probe"]))],
        )?;
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(
            std::fs::File::create(root.join("probe.parquet"))?,
            schema,
            Some(properties),
        )?;
        writer.write(&batch)?;
        writer.close()?;
        std::fs::write(root.join("probe.rss"), "result = select(col(id), probe);\n")?;
        let output_dir = root.join("out");
        let output = Command::new(binary)
            .current_dir(&root)
            .args(["--df", "--input", "probe=probe.parquet", "--result-dir"])
            .arg(&output_dir)
            .args(["--output-format", "parquet", "probe.rss"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Ok(false);
        }
        let metadata =
            crate::storage::ParquetInspector::metadata(&output_dir.join("result.parquet"))?;
        Ok(metadata.snappy && metadata.row_count == 1)
    })()
    .unwrap_or(false);
    let _ = std::fs::remove_dir_all(root);
    result
}
