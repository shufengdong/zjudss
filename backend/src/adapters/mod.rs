//! tensoreval 适配器是仿真内核与应用层之间的唯一边界。

mod contract;
mod import;
mod process;
mod stub;

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use sha2::Digest;

pub use contract::{ENGINE_CONTRACT_VERSION, EngineCapabilities};
pub use import::{
    EXECUTION_IMPORT_TABLES, ImportEntryCandidate, ImportInspectSpec, ImportInspection,
    ImportProfile, ImportSpec, ImportedModel, STATIC_MODEL_TABLES,
};
pub use process::{
    EngineCommand, EngineConfig, OutputLimits, ProcessEngine, build_command, validate_outputs,
};
pub use stub::StubEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Model,
    Solver,
    Io,
    Internal,
    Timeout,
}
impl ErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Solver => "solver",
            Self::Io => "io",
            Self::Internal => "internal",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdapterError {
    pub code: String,
    pub category: ErrorCategory,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Box<serde_json::Value>>,
}
impl AdapterError {
    pub fn new(
        code: impl Into<String>,
        category: ErrorCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            message: message.into(),
            diagnostics: None,
        }
    }
    pub fn with_diagnostics(mut self, diagnostics: serde_json::Value) -> Self {
        self.diagnostics = Some(Box::new(diagnostics));
        self
    }
}
impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AdapterError {}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InspectOutcome {
    pub ok: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct InspectSpec {
    pub input_tree: PathBuf,
    pub model_path: PathBuf,
    pub timeout: Duration,
}
impl InspectSpec {
    pub fn validate(&self) -> Result<(), AdapterError> {
        validate_tree_file(&self.input_tree, &self.model_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Rss,
    Bundle,
    Dataframe,
    Analysis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedInput {
    pub alias: String,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RunSpec {
    pub mode: RunMode,
    pub input_tree: PathBuf,
    pub script_path: PathBuf,
    pub named_inputs: Vec<NamedInput>,
    pub result_dir: PathBuf,
    pub timeout: Duration,
}
impl RunSpec {
    pub fn validate(&self) -> Result<(), AdapterError> {
        validate_tree_file(&self.input_tree, &self.script_path)?;
        if !self.result_dir.is_absolute() {
            return Err(AdapterError::new(
                "result_dir_invalid",
                ErrorCategory::Io,
                "result-dir 必须是绝对 staging 路径",
            ));
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&self.result_dir)
            && (metadata.file_type().is_symlink() || !metadata.is_dir())
        {
            return Err(AdapterError::new(
                "result_dir_invalid",
                ErrorCategory::Io,
                "result-dir 必须是普通 staging 目录",
            ));
        }
        if self.mode == RunMode::Bundle && !self.named_inputs.is_empty() {
            return Err(AdapterError::new(
                "named_input_not_allowed",
                ErrorCategory::Model,
                "bundle 运行不接受外部 named input",
            ));
        }
        let mut aliases = std::collections::HashSet::new();
        for input in &self.named_inputs {
            if !valid_alias(&input.alias) {
                return Err(AdapterError::new(
                    "input_alias_invalid",
                    ErrorCategory::Model,
                    format!("非法 input alias: {}", input.alias),
                ));
            }
            if !aliases.insert(input.alias.clone()) {
                return Err(AdapterError::new(
                    "input_alias_duplicate",
                    ErrorCategory::Model,
                    format!("重复 input alias: {}", input.alias),
                ));
            }
            validate_tree_file(&self.input_tree, &input.relative_path)?;
            if input.relative_path.extension().and_then(|v| v.to_str()) != Some("parquet") {
                return Err(AdapterError::new(
                    "input_format_invalid",
                    ErrorCategory::Model,
                    "命名输入必须是 Parquet",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct EngineDatasetOutput {
    pub name: String,
    pub path: PathBuf,
    pub metadata: crate::storage::ParquetMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedStream {
    pub retained: Vec<u8>,
    pub full_sha256: String,
    pub retained_sha256: String,
    pub total_bytes: u64,
    pub truncated: bool,
}

impl CapturedStream {
    pub fn empty() -> Self {
        let hash = hex::encode(sha2::Sha256::digest([]));
        Self {
            retained: Vec::new(),
            full_sha256: hash.clone(),
            retained_sha256: hash,
            total_bytes: 0,
            truncated: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineExecution {
    pub exit_code: i32,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
    pub datasets: Vec<EngineDatasetOutput>,
}

pub type EngineRunOutput = EngineExecution;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineFailureDiagnostic {
    pub code: String,
    pub exit_code: Option<i32>,
    pub stderr: CapturedStream,
    pub details: serde_json::Value,
}

fn valid_alias(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
pub(super) fn validate_tree_file(root: &Path, relative: &Path) -> Result<(), AdapterError> {
    if !root.is_absolute()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(AdapterError::new(
            "input_path_invalid",
            ErrorCategory::Model,
            "输入必须是任务私有树内的规范相对路径",
        ));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| AdapterError::new("input_tree_invalid", ErrorCategory::Io, e.to_string()))?;
    let target = root.join(relative);
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|e| AdapterError::new("input_missing", ErrorCategory::Io, e.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(AdapterError::new(
            "input_symlink_rejected",
            ErrorCategory::Model,
            "任务输入禁止符号链接",
        ));
    }
    let canonical = target
        .canonicalize()
        .map_err(|e| AdapterError::new("input_path_invalid", ErrorCategory::Io, e.to_string()))?;
    if !canonical.starts_with(canonical_root) {
        return Err(AdapterError::new(
            "input_path_escape",
            ErrorCategory::Model,
            "任务输入逃逸私有树",
        ));
    }
    Ok(())
}

#[async_trait]
pub trait SimEngine: Send + Sync {
    async fn inspect(&self, spec: InspectSpec) -> Result<InspectOutcome, AdapterError>;
    async fn run_dataframe(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError>;
    async fn run_bundle(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError>;
    async fn run_analysis(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError>;
    async fn inspect_model_import(
        &self,
        spec: ImportInspectSpec,
    ) -> Result<ImportInspection, AdapterError> {
        import::inspect_tree(&spec)
    }
    async fn import_model(&self, _spec: ImportSpec) -> Result<ImportedModel, AdapterError> {
        Err(AdapterError::new(
            "model_import_unavailable",
            ErrorCategory::Internal,
            "当前引擎未实现模型导入",
        ))
    }
}

pub fn resolve_data_dir() -> PathBuf {
    let dir = std::env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"));
    if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(dir)
    }
}

pub fn engine_from_env() -> Result<std::sync::Arc<dyn SimEngine>, AdapterError> {
    Ok(std::sync::Arc::new(ProcessEngine::from_env()?))
}

#[cfg(test)]
mod tests {
    use super::{AdapterError, ErrorCategory};

    #[test]
    fn adapter_error_keeps_diagnostics_without_inflating_every_result() {
        let error = AdapterError::new("engine_failed", ErrorCategory::Solver, "求解失败")
            .with_diagnostics(serde_json::json!({"iteration": 3}));

        assert!(std::mem::size_of::<AdapterError>() <= 104);
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "engine_failed",
                "category": "solver",
                "message": "求解失败",
                "diagnostics": {"iteration": 3}
            })
        );
    }
}
