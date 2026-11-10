//! 引擎退出后的最终结果集合、格式、压缩和容量合同校验。

use std::collections::HashSet;
use std::path::Path;

use crate::adapters::{AdapterError, CapturedStream, EngineDatasetOutput, EngineRunOutput};
use crate::storage::ParquetInspector;

use super::result_scan::{ScanPhase, output_error, scan_result_dir};

#[derive(Debug, Clone, Copy)]
/// 单次执行允许发布的文件数量与字节上限。
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

/// 最终扫描结果目录并返回已验证的 Snappy Parquet 数据集。
pub fn validate_outputs(
    result_dir: &Path,
    limits: OutputLimits,
) -> Result<EngineRunOutput, AdapterError> {
    let scanned = scan_result_dir(result_dir, limits, ScanPhase::Final)?;
    let mut names = HashSet::new();
    let mut datasets = Vec::new();
    for file in scanned.files {
        let path = file.path;
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
        let metadata = ParquetInspector::metadata(&path)
            .map_err(|error| output_error(error.code(), error.to_string()))?;
        if metadata.size_bytes != file.size {
            return Err(output_error(
                "result_file_invalid",
                format!("{name} 在最终扫描后发生变化"),
            ));
        }
        if !metadata.snappy {
            return Err(output_error(
                "result_compression_invalid",
                format!("{name} 不是 Snappy Parquet"),
            ));
        }
        datasets.push(EngineDatasetOutput {
            name,
            path,
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
