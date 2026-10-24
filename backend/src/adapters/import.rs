use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::{AdapterError, EngineDatasetOutput, ErrorCategory};

pub const STATIC_MODEL_TABLES: [&str; 21] = [
    "bus",
    "line",
    "linecode",
    "load",
    "transformer",
    "voltage_source",
    "motor",
    "transformer_winding",
    "transformer_impedance",
    "current_source",
    "generator",
    "ibr",
    "switch",
    "shunt",
    "capacitor",
    "reactor",
    "spectrum",
    "control_profile",
    "xy_curve",
    "storage",
    "para",
];

pub const EXECUTION_IMPORT_TABLES: [&str; 35] = [
    "bus",
    "line",
    "linecode",
    "load",
    "transformer",
    "voltage_source",
    "motor",
    "transformer_winding",
    "transformer_impedance",
    "current_source",
    "generator",
    "ibr",
    "switch",
    "shunt",
    "capacitor",
    "reactor",
    "spectrum",
    "control_profile",
    "xy_curve",
    "storage",
    "solution",
    "load_shape",
    "shape_assignment",
    "storage_controller",
    "switch_control",
    "tcc_curve",
    "fuse",
    "recloser",
    "relay",
    "monitor",
    "energy_meter",
    "fault",
    "para",
    "diagnostics",
    "events",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProfile {
    StaticDss,
    SingleSolveExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntryCandidate {
    pub id: String,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportInspection {
    pub entry_candidates: Vec<ImportEntryCandidate>,
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ImportInspectSpec {
    pub input_tree: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub input_tree: PathBuf,
    pub entry_candidate_id: String,
    pub result_dir: PathBuf,
    pub timeout: Duration,
}

impl ImportSpec {
    pub fn validate(&self) -> Result<(), AdapterError> {
        validate_tree(&self.input_tree)?;
        if self.entry_candidate_id.len() != 24
            || !self
                .entry_candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(AdapterError::new(
                "entry_candidate_invalid",
                ErrorCategory::Model,
                "入口候选 ID 无效",
            ));
        }
        if !self.result_dir.is_absolute() {
            return Err(AdapterError::new(
                "result_dir_invalid",
                ErrorCategory::Io,
                "import result-dir 必须是独立的绝对 staging 目录",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ImportedModel {
    pub profile: ImportProfile,
    pub dataframes: Vec<EngineDatasetOutput>,
    pub diagnostics: serde_json::Value,
}

pub(crate) fn inspect_tree(spec: &ImportInspectSpec) -> Result<ImportInspection, AdapterError> {
    validate_tree(&spec.input_tree)?;
    let files = dss_files(&spec.input_tree)?;
    if files.is_empty() {
        return Err(AdapterError::new(
            "model_import_entry_missing",
            ErrorCategory::Model,
            "没有可导入的 DSS 入口",
        ));
    }
    let available = files.iter().cloned().collect::<HashSet<_>>();
    let mut referenced = HashSet::new();
    for relative in &files {
        for dependency in dependencies(&spec.input_tree, relative)? {
            if !available.contains(&dependency) {
                return Err(AdapterError::new(
                    "missing_dependency",
                    ErrorCategory::Model,
                    format!("DSS 依赖不存在: {}", dependency.display()),
                ));
            }
            referenced.insert(dependency);
        }
    }
    let roots = files
        .into_iter()
        .filter(|relative| !referenced.contains(relative))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(AdapterError::new(
            "model_import_include_cycle",
            ErrorCategory::Model,
            "DSS include 图没有根入口",
        ));
    }
    let entry_candidates = roots
        .into_iter()
        .map(|relative| ImportEntryCandidate {
            id: candidate_id(&relative),
            display_path: portable(&relative),
        })
        .collect::<Vec<_>>();
    Ok(ImportInspection {
        diagnostics: serde_json::json!({
            "entryCount": entry_candidates.len(),
            "classifier": "adapter-v3"
        }),
        entry_candidates,
    })
}

pub(crate) fn resolve_entry(spec: &ImportSpec) -> Result<(PathBuf, ImportProfile), AdapterError> {
    spec.validate()?;
    let inspection = inspect_tree(&ImportInspectSpec {
        input_tree: spec.input_tree.clone(),
        timeout: spec.timeout,
    })?;
    let selected = inspection
        .entry_candidates
        .iter()
        .find(|candidate| candidate.id == spec.entry_candidate_id)
        .ok_or_else(|| {
            AdapterError::new(
                "entry_candidate_invalid",
                ErrorCategory::Model,
                "入口候选已失效",
            )
        })?;
    let relative = PathBuf::from(&selected.display_path);
    let profile = classify(&spec.input_tree, &relative)?;
    Ok((relative, profile))
}

fn classify(root: &Path, entry: &Path) -> Result<ImportProfile, AdapterError> {
    let mut pending = vec![entry.to_path_buf()];
    let mut visited = HashSet::new();
    let mut solve_count = 0usize;
    let mut unsupported = None;
    while let Some(relative) = pending.pop() {
        if !visited.insert(relative.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(root.join(&relative)).map_err(|error| {
            AdapterError::new(
                "model_import_read_failed",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
        for raw in source.lines() {
            let line = raw.trim().to_ascii_lowercase();
            if line.is_empty() || line.starts_with('!') || line.starts_with("//") {
                continue;
            }
            if line.split_whitespace().next() == Some("solve") || line.starts_with("solve(") {
                solve_count += 1;
            }
            if line.contains("harmonic")
                || line.starts_with("autoadd")
                || line.starts_with("relcalc")
                || line.contains("ds_") && line.contains("_dss_plan")
            {
                unsupported = Some(line);
            }
        }
        pending.extend(dependencies(root, &relative)?);
    }
    if solve_count > 1 || unsupported.is_some() {
        return Err(AdapterError::new(
            "model_import_unsupported",
            ErrorCategory::Model,
            "该模型生命周期不能完整投影为输入 DataFrame",
        )
        .with_diagnostics(serde_json::json!({
            "solveCount": solve_count,
            "unsupported": unsupported
        })));
    }
    Ok(if solve_count == 1 {
        ImportProfile::SingleSolveExecution
    } else {
        ImportProfile::StaticDss
    })
}

fn validate_tree(root: &Path) -> Result<(), AdapterError> {
    if !root.is_absolute() || !root.is_dir() {
        return Err(AdapterError::new(
            "input_tree_invalid",
            ErrorCategory::Io,
            "import 输入树必须是绝对目录",
        ));
    }
    Ok(())
}

fn dss_files(root: &Path) -> Result<Vec<PathBuf>, AdapterError> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("dss"))
            {
                output.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, &mut output).map_err(|error| {
        AdapterError::new(
            "model_import_read_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?;
    output.sort();
    Ok(output)
}

fn dependencies(root: &Path, relative: &Path) -> Result<Vec<PathBuf>, AdapterError> {
    let source = std::fs::read_to_string(root.join(relative)).map_err(|error| {
        AdapterError::new(
            "model_import_read_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut output = BTreeSet::new();
    for raw in source.lines() {
        let line = raw.trim();
        let Some((command, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !command.eq_ignore_ascii_case("redirect") && !command.eq_ignore_ascii_case("compile") {
            continue;
        }
        let value = rest
            .trim()
            .trim_matches(|character| matches!(character, '"' | '\'' | '(' | ')'));
        let dependency = normalize_dependency(&parent.join(value)).ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS include 路径不安全",
            )
        })?;
        output.insert(dependency);
    }
    Ok(output.into_iter().collect())
}

fn normalize_dependency(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn candidate_id(relative: &Path) -> String {
    let digest = hex::encode(Sha256::digest(portable(relative).as_bytes()));
    digest[..24].to_string()
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn order_model_outputs(
    outputs: Vec<EngineDatasetOutput>,
) -> Result<Vec<EngineDatasetOutput>, AdapterError> {
    let mut by_name = outputs
        .into_iter()
        .map(|output| (output.name.clone(), output))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(STATIC_MODEL_TABLES.len());
    for table in STATIC_MODEL_TABLES {
        ordered.push(by_name.remove(table).ok_or_else(|| {
            AdapterError::new(
                "model_import_output_missing",
                ErrorCategory::Model,
                format!("import 缺少标准表 {table}"),
            )
        })?);
    }
    if !by_name.is_empty() {
        return Err(AdapterError::new(
            "model_import_output_unexpected",
            ErrorCategory::Model,
            "import 返回了非模型 DataFrame",
        ));
    }
    Ok(ordered)
}
