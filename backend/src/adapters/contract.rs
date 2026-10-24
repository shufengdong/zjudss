use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use serde::Serialize;

use super::{AdapterError, EXECUTION_IMPORT_TABLES, ErrorCategory, STATIC_MODEL_TABLES};

pub const ENGINE_CONTRACT_VERSION: &str = "tensoreval-parquet-v3";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    pub contract_version: String,
    pub result_dir: bool,
    pub output_format_parquet: bool,
    pub named_parquet_inputs: bool,
    pub static_dss_import: bool,
    pub single_solve_import: bool,
    pub script_validation: bool,
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
        let static_dss_import =
            result_dir && output_format_parquet && model_import_smoke(binary, "import_dss");
        let single_solve_import = result_dir
            && output_format_parquet
            && model_import_smoke(binary, "import_dss_execution");
        let script_validation = script_validation_smoke();
        Ok(Self {
            contract_version: ENGINE_CONTRACT_VERSION.into(),
            result_dir,
            output_format_parquet,
            named_parquet_inputs,
            static_dss_import,
            single_solve_import,
            script_validation,
        })
    }

    fn require_parquet_contract(&self) -> Result<(), AdapterError> {
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

    pub fn require_v3(&self) -> Result<(), AdapterError> {
        self.require_parquet_contract()?;
        if !self.static_dss_import {
            return Err(AdapterError::new(
                "engine_static_model_import_unsupported",
                ErrorCategory::Internal,
                "tensoreval 缺少静态 DSS import profile",
            ));
        }
        if !self.single_solve_import {
            return Err(AdapterError::new(
                "engine_single_solve_import_unsupported",
                ErrorCategory::Internal,
                "tensoreval 缺少 single-Solve import profile",
            ));
        }
        if !self.script_validation {
            return Err(AdapterError::new(
                "engine_script_validation_unsupported",
                ErrorCategory::Internal,
                "adapter 缺少 V3 RSS 静态校验",
            ));
        }
        Ok(())
    }

    pub fn resource_features(&self) -> HashSet<String> {
        [
            ("named_parquet_inputs", self.named_parquet_inputs),
            ("static_dss_import", self.static_dss_import),
            ("single_solve_import", self.single_solve_import),
            ("script_validation", self.script_validation),
        ]
        .into_iter()
        .filter(|(_, available)| *available)
        .map(|(name, _)| name.to_string())
        .collect()
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

fn model_import_smoke(binary: &Path, function: &str) -> bool {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let root = std::env::temp_dir().join(format!(
        "zjudss-engine-import-probe-{}-{function}-{nonce}",
        std::process::id()
    ));
    let result = (|| -> Result<bool, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&root)?;
        write_probe_parquet(&root.join("probe.parquet"))?;
        let solve = if function == "import_dss_execution" {
            "Solve\n"
        } else {
            ""
        };
        std::fs::write(
            root.join("model.dss"),
            format!("New Circuit.probe basekv=10\n{solve}"),
        )?;
        let outputs = if function == "import_dss_execution" {
            EXECUTION_IMPORT_TABLES.as_slice()
        } else {
            STATIC_MODEL_TABLES.as_slice()
        };
        std::fs::write(
            root.join("import.rss"),
            format!("[{}] = {function}(\"model.dss\");\n", outputs.join(", ")),
        )?;
        let output_dir = root.join("out");
        let output = Command::new(binary)
            .current_dir(&root)
            .args(["--df", "--result-dir"])
            .arg(&output_dir)
            .args(["--output-format", "parquet", "import.rss"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Ok(false);
        }
        for name in outputs {
            let metadata = crate::storage::ParquetInspector::metadata(
                &output_dir.join(format!("{name}.parquet")),
            )?;
            if !metadata.snappy {
                return Ok(false);
            }
        }
        Ok(true)
    })()
    .unwrap_or(false);
    let _ = std::fs::remove_dir_all(root);
    result
}

fn write_probe_parquet(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(vec!["probe"]))],
    )?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(std::fs::File::create(path)?, schema, Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn script_validation_smoke() -> bool {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let root = std::env::temp_dir().join(format!(
        "zjudss-script-validation-probe-{}-{nonce}",
        std::process::id()
    ));
    let result = (|| -> Result<bool, AdapterError> {
        std::fs::create_dir_all(&root).map_err(|error| {
            AdapterError::new("engine_probe_failed", ErrorCategory::Io, error.to_string())
        })?;
        std::fs::write(root.join("probe.rss"), "result = select(col(id), bus);\n").map_err(
            |error| AdapterError::new("engine_probe_failed", ErrorCategory::Io, error.to_string()),
        )?;
        Ok(
            super::script::validate_script_source(&super::ScriptValidationSpec {
                input_tree: root.clone(),
                script_path: "probe.rss".into(),
                timeout: Duration::from_secs(1),
            })?
            .valid,
        )
    })()
    .unwrap_or(false);
    let _ = std::fs::remove_dir_all(root);
    result
}

pub(crate) fn binary_sha256(path: &Path) -> Result<String, AdapterError> {
    use sha2::{Digest, Sha256};

    let file = std::fs::File::open(path).map_err(|error| {
        AdapterError::new(
            "engine_binary_unreadable",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            AdapterError::new(
                "engine_binary_unreadable",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
