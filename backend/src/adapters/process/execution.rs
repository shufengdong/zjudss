//! 通用执行流程：准备隔离结果目录、启动进程、监控并完成最终扫描。

use tokio::process::Command;

use super::{ProcessEngine, build_command, control, validate_outputs};
use crate::adapters::{
    AdapterError, EngineFailureDiagnostic, EngineRunOutput, ErrorCategory, RunMode, RunSpec,
};

impl ProcessEngine {
    pub(super) fn ensure_binary_available(&self) -> Result<(), AdapterError> {
        ensure_binary_available(&self.config.bin)
    }

    pub(super) async fn execute(
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
        self.ensure_binary_available()?;
        let mut command = Command::new(&built.program);
        command.args(&built.args).current_dir(&built.cwd);
        let outcome = control::run_command(
            &mut command,
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

pub(super) fn ensure_binary_available(path: &std::path::Path) -> Result<(), AdapterError> {
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

pub(super) fn exit_error(output: &control::ProcessOutcome) -> AdapterError {
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
    stderr: &crate::adapters::CapturedStream,
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

pub(super) fn cleanup_result_dir(path: &std::path::Path) {
    let _ = std::fs::remove_dir_all(path);
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
