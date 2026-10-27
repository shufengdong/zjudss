use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::process::Command;

use crate::adapters::{AdapterError, CapturedStream, ErrorCategory};

use super::OutputLimits;
use super::capture::capture_stream;
use super::monitor::{monitor_result_dir, scan_result_limits};

pub(crate) struct ProcessOutcome {
    pub status: ExitStatus,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
}

pub(crate) async fn run_command(
    command: &mut Command,
    timeout: Duration,
    retained_limit: usize,
    monitored_result_dir: Option<(&Path, OutputLimits)>,
) -> Result<ProcessOutcome, AdapterError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn().map_err(|error| {
        AdapterError::new(
            "spawn_failed",
            ErrorCategory::Io,
            format!("无法启动 tensoreval: {error}"),
        )
    })?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or_else(|| {
        AdapterError::new(
            "engine_output_unavailable",
            ErrorCategory::Io,
            "无法捕获 tensoreval stdout",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AdapterError::new(
            "engine_output_unavailable",
            ErrorCategory::Io,
            "无法捕获 tensoreval stderr",
        )
    })?;
    let stdout_task = tokio::spawn(capture_stream(stdout, retained_limit));
    let stderr_task = tokio::spawn(capture_stream(stderr, retained_limit));
    let cleanup_target = monitored_result_dir.map(|(path, limits)| (path.to_path_buf(), limits));
    let mut guard = ProcessGroupGuard::new(pid, cleanup_target);

    let wait_result = if let Some((result_dir, limits)) = monitored_result_dir {
        tokio::time::timeout(timeout, async {
            tokio::select! {
                status = child.wait() => status.map_err(|error| AdapterError::new(
                    "engine_wait_failed", ErrorCategory::Io, error.to_string())),
                monitor = monitor_result_dir(result_dir, limits) => {
                    match monitor {
                        Ok(()) => unreachable!("result monitor only returns on failure"),
                        Err(error) => Err(error),
                    }
                }
            }
        })
        .await
    } else {
        tokio::time::timeout(timeout, async {
            child.wait().await.map_err(|error| {
                AdapterError::new("engine_wait_failed", ErrorCategory::Io, error.to_string())
            })
        })
        .await
    };

    let status = match wait_result {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            terminate_process_group(pid, &mut child).await;
            let stderr = join_capture(stderr_task)
                .await
                .unwrap_or_else(|_| CapturedStream::empty());
            let _ = join_capture(stdout_task).await;
            guard.cleanup();
            return Err(with_stderr(error, stderr));
        }
        Err(_) => {
            terminate_process_group(pid, &mut child).await;
            let stderr = join_capture(stderr_task)
                .await
                .unwrap_or_else(|_| CapturedStream::empty());
            let _ = join_capture(stdout_task).await;
            guard.cleanup();
            return Err(with_stderr(
                AdapterError::new(
                    "engine_timeout",
                    ErrorCategory::Timeout,
                    "tensoreval 执行超时",
                ),
                stderr,
            ));
        }
    };

    let stdout = join_capture(stdout_task).await?;
    let stderr = join_capture(stderr_task).await?;
    guard.disarm();
    Ok(ProcessOutcome {
        status,
        stdout,
        stderr,
    })
}

async fn join_capture(
    task: tokio::task::JoinHandle<Result<CapturedStream, AdapterError>>,
) -> Result<CapturedStream, AdapterError> {
    task.await.map_err(|error| {
        AdapterError::new(
            "engine_output_read_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?
}

fn with_stderr(mut error: AdapterError, stderr: CapturedStream) -> AdapterError {
    let mut details = error
        .diagnostics
        .take()
        .map(|value| *value)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = details.as_object_mut() {
        object.insert(
            "stderr".into(),
            serde_json::to_value(stderr).expect("captured stream serializes"),
        );
    }
    error.with_diagnostics(details)
}

async fn terminate_process_group(pid: Option<u32>, child: &mut tokio::process::Child) {
    kill_process_group(pid);
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn kill_process_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|value| i32::try_from(value).ok()) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

struct ProcessGroupGuard {
    pid: Option<u32>,
    cleanup_target: Option<(PathBuf, OutputLimits)>,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(pid: Option<u32>, cleanup_target: Option<(PathBuf, OutputLimits)>) -> Self {
        Self {
            pid,
            cleanup_target,
            armed: true,
        }
    }

    fn cleanup(&mut self) {
        if let Some((path, limits)) = &self.cleanup_target {
            let _ = scan_result_limits(path, *limits, false);
            let _ = std::fs::remove_dir_all(path);
        }
        self.armed = false;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            kill_process_group(self.pid);
            if let Some((path, limits)) = &self.cleanup_target {
                let _ = scan_result_limits(path, *limits, false);
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}
