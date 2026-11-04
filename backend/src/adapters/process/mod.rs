mod capture;
mod command;
mod control;
mod execution;
mod import;
mod monitor;
mod output;
mod result_scan;
mod task_file;

use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

use super::{
    AdapterError, CaseProjectionSpec, EngineCapabilities, EngineRunOutput, ErrorCategory,
    ImportSpec, ImportedModel, InspectOutcome, InspectSpec, ProjectedCase, RunMode, RunSpec,
    SimEngine,
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
        execution::ensure_binary_available(&bin)?;
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
            return Err(execution::exit_error(&output));
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
        self.import_model_process(spec).await
    }

    async fn project_case(&self, spec: CaseProjectionSpec) -> Result<ProjectedCase, AdapterError> {
        self.project_case_process(spec).await
    }
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
