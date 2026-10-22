use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::adapters::{AdapterError, ErrorCategory, RunMode, RunSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCommand {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub args: Vec<OsString>,
}

pub fn build_command(program: &Path, spec: &RunSpec) -> Result<EngineCommand, AdapterError> {
    spec.validate()?;
    let mut args = Vec::new();
    if spec.mode != RunMode::Bundle {
        args.push("--df".into());
    }
    for input in &spec.named_inputs {
        args.push("--input".into());
        args.push(format!("{}={}", input.alias, portable(&input.relative_path)?).into());
    }
    args.push("--result-dir".into());
    args.push(spec.result_dir.as_os_str().to_owned());
    args.push("--output-format".into());
    args.push("parquet".into());
    args.push(portable(&spec.script_path)?.into());
    Ok(EngineCommand {
        program: program.to_path_buf(),
        cwd: spec.input_tree.clone(),
        args,
    })
}

fn portable(path: &Path) -> Result<String, AdapterError> {
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(AdapterError::new(
            "input_path_invalid",
            ErrorCategory::Model,
            "输入必须是任务私有树内的规范相对路径",
        ));
    }
    path.to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "输入路径不是 UTF-8",
            )
        })
}
