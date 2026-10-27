use std::collections::HashSet;
use std::path::Path;

use crate::adapters::{
    AdapterError, CapturedStream, EngineDatasetOutput, EngineRunOutput, ErrorCategory,
};
use crate::storage::ParquetInspector;

use super::monitor::{limit_error, scan_result_limits};

#[derive(Debug, Clone, Copy)]
pub struct OutputLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}
impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            max_files: 100,
            max_file_bytes: 512 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

pub fn validate_outputs(
    result_dir: &Path,
    limits: OutputLimits,
) -> Result<EngineRunOutput, AdapterError> {
    scan_result_limits(result_dir, limits, false)?;
    let root = result_dir
        .canonicalize()
        .map_err(|error| output_error("result_dir_invalid", error.to_string()))?;
    let mut entries: Vec<_> = std::fs::read_dir(&root)
        .map_err(|error| output_error("result_dir_invalid", error.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|error| output_error("result_dir_invalid", error.to_string()))?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() > limits.max_files {
        return Err(limit_error(
            "result_file_limit",
            "结果文件数超限",
            entries.len() as u64,
            limits.max_files as u64,
        ));
    }
    let mut total = 0_u64;
    let mut names = HashSet::new();
    let mut datasets = Vec::new();
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| output_error("result_file_invalid", error.to_string()))?;
        if file_type.is_symlink() {
            return Err(output_error(
                "result_symlink_rejected",
                "结果目录禁止符号链接",
            ));
        }
        if !file_type.is_file() {
            return Err(output_error(
                "result_file_invalid",
                "结果目录只允许普通 Parquet 文件",
            ));
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("parquet") {
            return Err(output_error(
                "result_format_invalid",
                "结果目录包含非 Parquet 文件",
            ));
        }
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| output_error("result_name_invalid", "结果名不是 UTF-8"))?
            .to_string();
        if !valid_name(&name) {
            return Err(output_error(
                "result_name_invalid",
                format!("非法结果变量名: {name}"),
            ));
        }
        register_name(&mut names, &name)?;
        let canonical = path
            .canonicalize()
            .map_err(|error| output_error("result_file_invalid", error.to_string()))?;
        if !canonical.starts_with(&root) {
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
        let metadata = ParquetInspector::metadata(&canonical)
            .map_err(|error| output_error(error.code(), error.to_string()))?;
        if !metadata.snappy {
            return Err(output_error(
                "result_compression_invalid",
                format!("{name} 不是 Snappy Parquet"),
            ));
        }
        datasets.push(EngineDatasetOutput {
            name,
            path: canonical,
            metadata,
        });
    }
    if datasets.is_empty() {
        return Err(output_error(
            "result_empty",
            "tensoreval 未生成 Parquet 结果",
        ));
    }
    Ok(EngineRunOutput {
        exit_code: 0,
        stdout: CapturedStream::empty(),
        stderr: CapturedStream::empty(),
        datasets,
    })
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn register_name(names: &mut HashSet<String>, name: &str) -> Result<(), AdapterError> {
    if names.insert(name.to_ascii_lowercase()) {
        Ok(())
    } else {
        Err(output_error(
            "result_name_duplicate",
            format!("重复结果名: {name}"),
        ))
    }
}
fn output_error(code: impl Into<String>, message: impl Into<String>) -> AdapterError {
    AdapterError::new(code, ErrorCategory::Io, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn result_names_are_case_insensitively_unique() {
        let mut names = HashSet::new();
        register_name(&mut names, "Voltage").unwrap();
        assert_eq!(
            register_name(&mut names, "voltage").unwrap_err().code,
            "result_name_duplicate"
        );
    }
}
