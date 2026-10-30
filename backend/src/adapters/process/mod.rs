mod capture;
mod command;
mod control;
mod monitor;
mod output;

use std::path::PathBuf;
use std::time::Duration;
use std::{fs::OpenOptions, io::Write};

use async_trait::async_trait;
use tokio::process::Command;

use super::{
    AdapterError, CaseProjectionSpec, EXECUTION_IMPORT_TABLES, EngineCapabilities,
    EngineFailureDiagnostic, EngineRunOutput, ErrorCategory, ImportProfile, ImportSpec,
    ImportedModel, InspectOutcome, InspectSpec, ProjectedCase, RunMode, RunSpec,
    STATIC_IMPORT_OUTPUTS, SimEngine,
};
pub use command::{EngineCommand, build_command};
pub use output::{OutputLimits, validate_outputs};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub bin: PathBuf,
    pub binary_sha256: String,
    pub default_timeout: Duration,
    pub process_output_max_bytes: usize,
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
            process_output_max_bytes: 1024 * 1024,
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
        let mut config = EngineConfig::probe(bin, Duration::from_millis(timeout))?;
        config.process_output_max_bytes = env_usize(
            "ENGINE_PROCESS_OUTPUT_MAX_BYTES",
            config.process_output_max_bytes,
        )?;
        config.output_limits = OutputLimits {
            max_files: env_usize("RESULT_MAX_FILES", config.output_limits.max_files)?,
            max_file_bytes: env_u64("RESULT_MAX_FILE_BYTES", config.output_limits.max_file_bytes)?,
            max_total_bytes: env_u64(
                "RESULT_MAX_TOTAL_BYTES",
                config.output_limits.max_total_bytes,
            )?,
        };
        Ok(Self::new(config))
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
        prepare_result_dir(&spec.result_dir)?;
        let timeout = if spec.timeout.is_zero() {
            self.config.default_timeout
        } else {
            spec.timeout
        };
        self.ensure_binary_available()?;
        let mut command = Command::new(&built.program);
        command.args(&built.args).current_dir(&built.cwd);
        let outcome = control::run_command(
            &mut command,
            timeout,
            self.config.process_output_max_bytes,
            Some((&spec.result_dir, self.config.output_limits)),
        )
        .await?;
        if !outcome.status.success() {
            cleanup_result_dir(&spec.result_dir);
            return Err(exit_error(&outcome));
        }
        let validated = match validate_outputs(&spec.result_dir, self.config.output_limits) {
            Ok(output) => output,
            Err(error) => {
                cleanup_result_dir(&spec.result_dir);
                return Err(with_captured_stderr(
                    error,
                    &outcome.stderr,
                    outcome.status.code(),
                ));
            }
        };
        Ok(EngineRunOutput {
            exit_code: outcome.status.code().unwrap_or_default(),
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            datasets: validated.datasets,
        })
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
            .current_dir(&spec.input_tree);
        let timeout = if spec.timeout.is_zero() {
            self.config.default_timeout
        } else {
            spec.timeout
        };
        let output = control::run_command(
            &mut command,
            timeout,
            self.config.process_output_max_bytes,
            None,
        )
        .await?;
        if !output.status.success() {
            return Err(exit_error(&output));
        }
        Ok(InspectOutcome {
            ok: true,
            summary: String::from_utf8_lossy(&output.stdout.retained)
                .trim()
                .to_string(),
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
            ImportProfile::StaticDss => ("import_dss", STATIC_IMPORT_OUTPUTS.as_slice()),
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
            let model_names = STATIC_IMPORT_OUTPUTS
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
                "tableCount": STATIC_IMPORT_OUTPUTS.len()
            }),
        })
    }

    async fn project_case(&self, spec: CaseProjectionSpec) -> Result<ProjectedCase, AdapterError> {
        spec.validate()?;
        let source = spec
            .source_entry
            .to_str()
            .expect("validated UTF-8 case source")
            .replace('\\', "/");
        let source_literal = serde_json::to_string(&source).map_err(|error| {
            AdapterError::new(
                "case_projection_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let outputs = spec
            .outputs
            .iter()
            .map(|output| output.alias.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| {
            AdapterError::new(
                "case_projection_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let script_name = format!(".zjudss-case-projection-{}.rss", hex::encode(nonce));
        let script_path = spec.input_tree.join(&script_name);
        let mut script = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
            .map_err(|error| {
                AdapterError::new(
                    "case_projection_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        let script_guard = RemoveFileGuard::new(script_path);
        script
            .write_all(format!("[{outputs}] = {}({source_literal});\n", spec.projector).as_bytes())
            .map_err(|error| {
                AdapterError::new(
                    "case_projection_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                )
            })?;
        drop(script);
        let expected = spec.outputs;
        let result_dir = spec.result_dir.clone();
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
        let cleanup = script_guard.cleanup();
        let output = match (executed, cleanup) {
            (Ok(output), Ok(())) => output,
            (Ok(_), Err(error)) => {
                cleanup_result_dir(&result_dir);
                return Err(AdapterError::new(
                    "case_projection_script_failed",
                    ErrorCategory::Io,
                    error.to_string(),
                ));
            }
            (Err(error), Ok(())) => return Err(error),
            (Err(mut error), Err(cleanup_error)) => {
                error.message = format!(
                    "{}; 案例投影临时脚本清理失败: {cleanup_error}",
                    error.message
                );
                return Err(error);
            }
        };
        match super::import::order_projection_outputs(output.datasets, &expected) {
            Ok(projected) => Ok(projected),
            Err(error) => {
                cleanup_result_dir(&result_dir);
                Err(error)
            }
        }
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

fn exit_error(output: &control::ProcessOutcome) -> AdapterError {
    const MAX_ERROR_OUTPUT_CHARS: usize = 16_384;
    let stderr = String::from_utf8_lossy(&output.stderr.retained);
    let stdout = String::from_utf8_lossy(&output.stdout.retained);
    let raw_output = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let lower = raw_output.to_ascii_lowercase();
    let (code, category) = if lower.contains("converge") || lower.contains("solve") {
        ("solver_error", ErrorCategory::Solver)
    } else if lower.contains("parse") || lower.contains("import") || lower.contains("invalid") {
        ("model_error", ErrorCategory::Model)
    } else {
        ("engine_error", ErrorCategory::Internal)
    };
    let diagnostic = EngineFailureDiagnostic {
        code: code.into(),
        exit_code: output.status.code(),
        stderr: output.stderr.clone(),
        details: serde_json::json!({"stdout": output.stdout}),
    };
    AdapterError::new(
        code,
        category,
        bounded_text(raw_output, MAX_ERROR_OUTPUT_CHARS)
            .unwrap_or_else(|| "tensoreval 执行失败".into()),
    )
    .with_diagnostics(serde_json::to_value(diagnostic).expect("engine diagnostic serializes"))
}

fn with_captured_stderr(
    mut error: AdapterError,
    stderr: &super::CapturedStream,
    exit_code: Option<i32>,
) -> AdapterError {
    let mut details = error
        .diagnostics
        .take()
        .map(|value| *value)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = details.as_object_mut() {
        object.insert("exitCode".into(), serde_json::json!(exit_code));
        object.insert(
            "stderr".into(),
            serde_json::to_value(stderr).expect("captured stderr serializes"),
        );
    }
    error.with_diagnostics(details)
}

fn prepare_result_dir(path: &std::path::Path) -> Result<(), AdapterError> {
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|error| {
            AdapterError::new(
                "result_dir_create_failed",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
    }
    std::fs::create_dir_all(path).map_err(|error| {
        AdapterError::new(
            "result_dir_create_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })
}

fn cleanup_result_dir(path: &std::path::Path) {
    let _ = std::fs::remove_dir_all(path);
}

fn env_usize(name: &'static str, default: usize) -> Result<usize, AdapterError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                AdapterError::new(
                    "engine_config_invalid",
                    ErrorCategory::Internal,
                    format!("{name} 必须是大于零的整数"),
                )
            }),
        Err(_) => Ok(default),
    }
}

fn env_u64(name: &'static str, default: u64) -> Result<u64, AdapterError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                AdapterError::new(
                    "engine_config_invalid",
                    ErrorCategory::Internal,
                    format!("{name} 必须是大于零的整数"),
                )
            }),
        Err(_) => Ok(default),
    }
}

fn bounded_text(value: &str, limit: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut characters = value.chars();
    let mut bounded = characters.by_ref().take(limit).collect::<String>();
    if characters.next().is_some() {
        bounded.push('…');
    }
    Some(bounded)
}
