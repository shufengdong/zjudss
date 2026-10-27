use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::adapters::{AdapterError, ErrorCategory};

use super::OutputLimits;

pub(crate) async fn monitor_result_dir(
    result_dir: &Path,
    limits: OutputLimits,
) -> Result<(), AdapterError> {
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        scan_result_limits(result_dir, limits, true)?;
    }
}

pub(crate) fn scan_result_limits(
    result_dir: &Path,
    limits: OutputLimits,
    monitoring: bool,
) -> Result<(), AdapterError> {
    let metadata = std::fs::symlink_metadata(result_dir).map_err(|error| {
        if monitoring {
            monitor_error(error)
        } else {
            output_error("result_dir_invalid", error.to_string())
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(output_error(
            "result_symlink_rejected",
            "结果目录禁止符号链接",
        ));
    }
    if !metadata.is_dir() {
        return Err(output_error(
            "result_dir_invalid",
            "result-dir 必须是普通目录",
        ));
    }
    let canonical_root = result_dir.canonicalize().map_err(|error| {
        if monitoring {
            monitor_error(error)
        } else {
            output_error("result_dir_invalid", error.to_string())
        }
    })?;
    let entries = std::fs::read_dir(result_dir).map_err(|error| {
        if monitoring {
            monitor_error(error)
        } else {
            output_error("result_dir_invalid", error.to_string())
        }
    })?;
    let mut count = 0_usize;
    let mut total = 0_u64;
    for entry in entries {
        let entry = entry.map_err(|error| {
            if monitoring {
                monitor_error(error)
            } else {
                output_error("result_file_invalid", error.to_string())
            }
        })?;
        count = count.saturating_add(1);
        if count > limits.max_files {
            return Err(limit_error(
                "result_file_limit",
                "结果文件数超限",
                count as u64,
                limits.max_files as u64,
            ));
        }
        let file_type = entry.file_type().map_err(|error| {
            if monitoring {
                monitor_error(error)
            } else {
                output_error("result_file_invalid", error.to_string())
            }
        })?;
        if file_type.is_symlink() {
            return Err(output_error(
                "result_symlink_rejected",
                "结果目录禁止符号链接",
            ));
        }
        if !file_type.is_file() {
            return Err(output_error(
                "result_file_invalid",
                "结果目录只允许普通文件",
            ));
        }
        let path = entry.path();
        let canonical = path
            .canonicalize()
            .map_err(|error| output_error("result_file_invalid", error.to_string()))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(output_error(
                "result_path_escape",
                "结果文件逃逸 result-dir",
            ));
        }
        let size = std::fs::metadata(&canonical)
            .map_err(|error| output_error("result_file_invalid", error.to_string()))?
            .len();
        if size > limits.max_file_bytes {
            return Err(limit_error(
                "result_file_too_large",
                "结果文件超过单文件限制",
                size,
                limits.max_file_bytes,
            ));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| output_error("result_total_too_large", "结果总大小溢出"))?;
        if total > limits.max_total_bytes {
            return Err(limit_error(
                "result_total_too_large",
                "结果总大小超限",
                total,
                limits.max_total_bytes,
            ));
        }
    }
    Ok(())
}

fn monitor_error(error: std::io::Error) -> AdapterError {
    AdapterError::new(
        "result_monitor_failed",
        ErrorCategory::Io,
        format!("结果目录监控失败: {error}"),
    )
}

pub(crate) fn output_error(code: &'static str, message: impl Into<String>) -> AdapterError {
    AdapterError::new(code, ErrorCategory::Io, message)
}

pub(crate) fn limit_error(
    code: &'static str,
    message: &'static str,
    actual: u64,
    limit: u64,
) -> AdapterError {
    output_error(code, message).with_diagnostics(json!({
        "actual": actual,
        "limit": limit,
    }))
}
