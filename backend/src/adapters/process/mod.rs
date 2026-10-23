mod command;
mod output;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{
    AdapterError, EngineCapabilities, EngineRunOutput, ErrorCategory, ImportProfile, ImportSpec,
    ImportedModel, InspectOutcome, InspectSpec, RunMode, RunSpec, STATIC_MODEL_TABLES, SimEngine,
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
        let before = super::contract::binary_sha256(&bin)?;
        let capabilities = EngineCapabilities::probe(&bin)?;
        capabilities.require_v3()?;
        let binary_sha256 = super::contract::binary_sha256(&bin)?;
        if before != binary_sha256 {
            return Err(AdapterError::new(
                "engine_binary_changed",
                ErrorCategory::Io,
                "tensoreval 在启动探测期间发生变化，请重启服务",
            ));
        }
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

    fn verify_binary(&self) -> Result<(), AdapterError> {
        let current = super::contract::binary_sha256(&self.config.bin)?;
        if current != self.config.binary_sha256 {
            return Err(AdapterError::new(
                "engine_binary_changed",
                ErrorCategory::Io,
                "tensoreval 已被替换，请重启服务后重试",
            ));
        }
        Ok(())
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
        self.verify_binary()?;
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
        self.verify_binary()?;
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
        let function = match profile {
            ImportProfile::StaticDss => "import_dss",
            ImportProfile::SingleSolveExecution => "import_dss_execution",
        };
        let entry = entry.to_str().ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS 入口不是 UTF-8",
            )
        })?;
        let script_name = ".zjudss-model-import.rss";
        let outputs = STATIC_MODEL_TABLES.join(", ");
        let entry_literal = serde_json::to_string(&entry.replace('\\', "/")).map_err(|error| {
            AdapterError::new(
                "model_import_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        std::fs::write(
            spec.input_tree.join(script_name),
            format!("[{outputs}] = {function}({entry_literal});\n"),
        )
        .map_err(|error| {
            AdapterError::new(
                "model_import_script_failed",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
        let output = self
            .execute(
                RunSpec {
                    mode: RunMode::Dataframe,
                    input_tree: spec.input_tree,
                    script_path: PathBuf::from(script_name),
                    named_inputs: Vec::new(),
                    result_dir: spec.result_dir,
                    timeout: spec.timeout,
                },
                &[RunMode::Dataframe],
            )
            .await?;
        Ok(ImportedModel {
            profile,
            dataframes: super::import::order_model_outputs(output.datasets)?,
            diagnostics: serde_json::json!({
                "importMethod": function,
                "tableCount": STATIC_MODEL_TABLES.len()
            }),
        })
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
