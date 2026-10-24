mod command;
mod output;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::{fs::OpenOptions, io::Write};

use async_trait::async_trait;
use tokio::process::Command;

use super::{
    AdapterError, EXECUTION_IMPORT_TABLES, EngineCapabilities, EngineRunOutput, ErrorCategory,
    ImportProfile, ImportSpec, ImportedModel, InspectOutcome, InspectSpec, RunMode, RunSpec,
    STATIC_MODEL_TABLES, SimEngine,
};
pub use command::{EngineCommand, build_command};
pub use output::{OutputLimits, validate_outputs};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub bin: PathBuf,
    pub binary_sha256: String,
    pub default_timeout: Duration,
    pub output_limits: OutputLimits,
    pub capabilities: EngineCapabilities,
}

impl EngineConfig {
    pub fn probe(bin: PathBuf, default_timeout: Duration) -> Result<Self, AdapterError> {
        ensure_binary_available(&bin)?;
        let capabilities = EngineCapabilities::probe(&bin)?;
        capabilities.require_v3()?;
        let binary_sha256 = super::contract::binary_sha256(&bin)?;
        Ok(Self {
            bin,
            binary_sha256,
            default_timeout,
            output_limits: OutputLimits::default(),
            capabilities,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessEngine {
    config: EngineConfig,
}

struct RemoveFileGuard {
    path: PathBuf,
    armed: bool,
}

impl RemoveFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn cleanup(mut self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for RemoveFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl ProcessEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Result<Self, AdapterError> {
        let bin = std::env::var_os("TENSOREVAL_BIN")
            .map(PathBuf::from)
            .ok_or_else(|| {
                AdapterError::new(
                    "engine_binary_required",
                    ErrorCategory::Io,
                    "必须设置 TENSOREVAL_BIN；生产模式不回退 stub",
                )
            })?;
        let timeout = std::env::var("TENSOREVAL_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120_000);
        Ok(Self::new(EngineConfig::probe(
            bin,
            Duration::from_millis(timeout),
        )?))
    }

    pub fn capabilities(&self) -> &EngineCapabilities {
        &self.config.capabilities
    }

    pub fn binary_sha256(&self) -> &str {
        &self.config.binary_sha256
    }

    fn ensure_binary_available(&self) -> Result<(), AdapterError> {
        ensure_binary_available(&self.config.bin)
    }

    async fn execute(
        &self,
        spec: RunSpec,
        allowed: &[RunMode],
    ) -> Result<EngineRunOutput, AdapterError> {
        if !allowed.contains(&spec.mode) {
            return Err(AdapterError::new(
                "run_mode_invalid",
                ErrorCategory::Model,
                "适配器入口与运行模式不匹配",
            ));
        }
        let built = build_command(&self.config.bin, &spec)?;
        std::fs::create_dir_all(&spec.result_dir).map_err(|error| {
            AdapterError::new(
                "result_dir_create_failed",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
        let timeout = if spec.timeout.is_zero() {
            self.config.default_timeout
        } else {
            spec.timeout
        };
        self.ensure_binary_available()?;
        let mut command = Command::new(&built.program);
        command
            .args(&built.args)
            .current_dir(&built.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| {
                AdapterError::new("timeout", ErrorCategory::Timeout, "tensoreval 执行超时")
            })?
            .map_err(|error| {
                AdapterError::new(
                    "spawn_failed",
                    ErrorCategory::Io,
                    format!("无法启动 tensoreval: {error}"),
                )
            })?;
        if !output.status.success() {
            return Err(exit_error(&output));
        }
        validate_outputs(&spec.result_dir, self.config.output_limits)
    }
}

#[async_trait]
impl SimEngine for ProcessEngine {
    async fn inspect(&self, spec: InspectSpec) -> Result<InspectOutcome, AdapterError> {
        spec.validate()?;
        let model = spec.model_path.to_str().ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "模型相对路径不是 UTF-8",
            )
        })?;
        self.ensure_binary_available()?;
        let mut command = Command::new(&self.config.bin);
        command
            .args(["--dss-to-rss", "--check", "--study", "snapshot", model])
            .current_dir(&spec.input_tree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let timeout = if spec.timeout.is_zero() {
            self.config.default_timeout
        } else {
            spec.timeout
        };
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| {
                AdapterError::new("timeout", ErrorCategory::Timeout, "tensoreval 检查超时")
            })?
            .map_err(|error| {
                AdapterError::new("spawn_failed", ErrorCategory::Io, error.to_string())
            })?;
        if !output.status.success() {
            return Err(exit_error(&output));
        }
        Ok(InspectOutcome {
            ok: true,
            summary: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            diagnostics: None,
        })
    }

    async fn run_dataframe(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.execute(spec, &[RunMode::Rss, RunMode::Dataframe])
            .await
    }
    async fn run_bundle(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.execute(spec, &[RunMode::Bundle]).await
    }
    async fn run_analysis(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.execute(spec, &[RunMode::Analysis]).await
    }
    async fn import_model(&self, spec: ImportSpec) -> Result<ImportedModel, AdapterError> {
        let (entry, profile) = super::import::resolve_entry(&spec)?;
        let (function, output_names): (&str, &[&str]) = match profile {
            ImportProfile::StaticDss => ("import_dss", STATIC_MODEL_TABLES.as_slice()),
            ImportProfile::SingleSolveExecution => {
                ("import_dss_execution", EXECUTION_IMPORT_TABLES.as_slice())
            }
        };
        let entry = entry.to_str().ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS 入口不是 UTF-8",
            )
        })?;
        let entry = entry.replace('\\', "/");
        if entry
            .chars()
            .any(|character| character == '"' || character.is_control())
        {
            return Err(AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS 入口包含不安全字符",
            ));
        }
        let entry_literal = format!("\"{entry}\"");
        let mut wrapper_nonce = [0_u8; 16];
        getrandom::fill(&mut wrapper_nonce).map_err(|error| {
            AdapterError::new(
                "model_import_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let nonce = hex::encode(wrapper_nonce);
        let wrapper_name = format!(".zjudss-model-entry-{nonce}.dss");
        let wrapper_path = spec.input_tree.join(&wrapper_name);
        let mut wrapper = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&wrapper_path)
            .map_err(|error| {
                AdapterError::new(
                    "model_import_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        wrapper
            .write_all(format!("Redirect {entry_literal}\n").as_bytes())
            .map_err(|error| {
                let _ = std::fs::remove_file(&wrapper_path);
                AdapterError::new(
                    "model_import_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        drop(wrapper);
        let wrapper_guard = RemoveFileGuard::new(wrapper_path);
        let script_name = format!(".zjudss-model-import-{nonce}.rss");
        let script_path = spec.input_tree.join(&script_name);
        let outputs = output_names.join(", ");
        let wrapper_literal = serde_json::to_string(&wrapper_name).map_err(|error| {
            AdapterError::new(
                "model_import_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let mut script = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(|error| {
                AdapterError::new(
                    "model_import_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        let script_guard = RemoveFileGuard::new(script_path);
        script
            .write_all(format!("[{outputs}] = {function}({wrapper_literal});\n").as_bytes())
            .map_err(|error| {
                AdapterError::new(
                    "model_import_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        drop(script);
        let executed = self
            .execute(
                RunSpec {
                    mode: RunMode::Dataframe,
                    input_tree: spec.input_tree,
                    script_path: PathBuf::from(&script_name),
                    named_inputs: Vec::new(),
                    result_dir: spec.result_dir,
                    timeout: spec.timeout,
                },
                &[RunMode::Dataframe],
            )
            .await;
        let cleanup = script_guard
            .cleanup()
            .and_then(|()| wrapper_guard.cleanup());
        let output = match (executed, cleanup) {
            (Ok(output), Ok(())) => output,
            (Ok(_), Err(error)) => {
                return Err(AdapterError::new(
                    "model_import_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                ));
            }
            (Err(error), Ok(())) => return Err(error),
            (Err(mut error), Err(cleanup_error)) => {
                error.message = format!("{}; 导入临时文件清理失败: {cleanup_error}", error.message);
                return Err(error);
            }
        };
        let datasets = if profile == ImportProfile::SingleSolveExecution {
            let model_names = STATIC_MODEL_TABLES
                .into_iter()
                .collect::<std::collections::HashSet<_>>();
            output
                .datasets
                .into_iter()
                .filter(|dataset| model_names.contains(dataset.name.as_str()))
                .collect()
        } else {
            output.datasets
        };
        Ok(ImportedModel {
            profile,
            dataframes: super::import::order_model_outputs(datasets)?,
            diagnostics: serde_json::json!({
                "importMethod": function,
                "tableCount": STATIC_MODEL_TABLES.len()
            }),
        })
    }
}

fn ensure_binary_available(path: &std::path::Path) -> Result<(), AdapterError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(AdapterError::new(
            "engine_binary_unavailable",
            ErrorCategory::Io,
            "TENSOREVAL_BIN 必须指向普通文件",
        )),
        Err(error) => Err(AdapterError::new(
            "engine_binary_unavailable",
            ErrorCategory::Io,
            format!("tensoreval 不存在或不可访问: {error}"),
        )),
    }
}

fn exit_error(output: &std::process::Output) -> AdapterError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_ascii_lowercase();
    let (code, category) = if lower.contains("converge") || lower.contains("solve") {
        ("solver_error", ErrorCategory::Solver)
    } else if lower.contains("parse") || lower.contains("import") || lower.contains("invalid") {
        ("model_error", ErrorCategory::Model)
    } else {
        ("engine_error", ErrorCategory::Internal)
    };
    AdapterError::new(
        code,
        category,
        stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("tensoreval 执行失败")
            .trim(),
    )
    .with_diagnostics(serde_json::json!({"exitCode":output.status.code()}))
}
