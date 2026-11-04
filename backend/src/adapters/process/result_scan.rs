use std::path::{Path, PathBuf};

use serde_json::json;

use crate::adapters::{AdapterError, ErrorCategory};

use super::OutputLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanPhase {
    Monitoring,
    Final,
}

#[derive(Debug)]
pub(crate) struct ScannedResultFile {
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
}

#[derive(Debug)]
pub(crate) struct ScannedResultDir {
    pub(crate) files: Vec<ScannedResultFile>,
}

pub(crate) fn scan_result_dir(
    result_dir: &Path,
    limits: OutputLimits,
    phase: ScanPhase,
) -> Result<ScannedResultDir, AdapterError> {
    let metadata =
        std::fs::symlink_metadata(result_dir).map_err(|error| directory_error(phase, error))?;
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
    let canonical_root = result_dir
        .canonicalize()
        .map_err(|error| directory_error(phase, error))?;
    let entries = std::fs::read_dir(result_dir).map_err(|error| directory_error(phase, error))?;
    let mut files = Vec::new();
    let mut total = 0_u64;
    for entry in entries {
        let entry = entry.map_err(|error| entry_error(phase, error))?;
        let count = files.len().saturating_add(1);
        if count > limits.max_files {
            return Err(limit_error(
                "result_file_limit",
                "结果文件数超限",
                count as u64,
                limits.max_files as u64,
            ));
        }
        let file_type = entry
            .file_type()
            .map_err(|error| entry_error(phase, error))?;
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
        let canonical = entry
            .path()
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
        files.push(ScannedResultFile {
            path: canonical,
            size,
        });
    }
    files.sort_by(|left, right| left.path.file_name().cmp(&right.path.file_name()));
    Ok(ScannedResultDir { files })
}

fn directory_error(phase: ScanPhase, error: std::io::Error) -> AdapterError {
    match phase {
        ScanPhase::Monitoring => monitor_error(error),
        ScanPhase::Final => output_error("result_dir_invalid", error.to_string()),
    }
}

fn entry_error(phase: ScanPhase, error: std::io::Error) -> AdapterError {
    match phase {
        ScanPhase::Monitoring => monitor_error(error),
        ScanPhase::Final => output_error("result_file_invalid", error.to_string()),
    }
}

fn monitor_error(error: std::io::Error) -> AdapterError {
    AdapterError::new(
        "result_monitor_failed",
        ErrorCategory::Io,
        format!("结果目录监控失败: {error}"),
    )
}

pub(crate) fn output_error(code: impl Into<String>, message: impl Into<String>) -> AdapterError {
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
