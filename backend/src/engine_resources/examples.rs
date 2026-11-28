//! 案例 manifest、依赖图和受限源文件的加载与验证。

use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::EngineResourceError;
use super::projections::{CaseProjectionProfile, CaseProjectionRegistry};
use super::snapshot::{ResourceLimits, SourceFileSpec};
use super::tree::{hash_bounded_file_with_size, limit_error, read_bounded_file, safe_file};

const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: [u32; 2] = [2, 3];
const SUPPORTED_BUNDLE_ARTIFACT: &str = "mems.dss-rss-bundle";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// 从受限资源树加载并带内容摘要的单个引擎案例。
pub struct EngineExample {
    pub id: String,
    pub title: String,
    pub source_kind: String,
    pub category_path: Vec<String>,
    pub bundle_path: String,
    pub entry_path: Option<String>,
    pub manifest: Option<ExampleManifest>,
    pub file_count: usize,
    pub size_bytes: u64,
    pub tree_hash: String,
    pub importable: bool,
    pub diagnostics: Vec<String>,
    #[serde(skip)]
    pub files: Vec<ExampleSourceFile>,
}

impl EngineExample {
    pub fn matches_catalog_query(&self, query: &str, study_type: Option<&str>) -> bool {
        let manifest = self.manifest.as_ref();
        if let Some(expected) = study_type {
            let expected = expected.trim().to_lowercase();
            if !manifest.is_some_and(|manifest| manifest.study.kind.to_lowercase() == expected) {
                return false;
            }
        }

        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let contains = |value: &str| value.to_lowercase().contains(&query);
        contains(&self.title)
            || contains(&self.bundle_path)
            || manifest.is_some_and(|manifest| {
                contains(&manifest.study.kind)
                    || manifest.study.solver.as_deref().is_some_and(contains)
                    || contains(&manifest.study.compatibility)
                    || manifest.study.results.iter().any(|result| contains(result))
                    || contains(&manifest.source.entry)
            })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleManifest {
    pub schema_version: u32,
    pub artifact: String,
    pub generator: String,
    pub psa_contract_version: String,
    pub entry: String,
    pub study: ExampleStudy,
    pub source: ExampleSource,
    pub projection: ExampleProjection,
    pub files: Vec<ExampleManifestFile>,
    pub inputs: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleProjection {
    pub source_function: String,
    pub profile: String,
    pub projector: String,
    pub target_function: String,
    pub managed_entry: String,
    pub status: String,
    pub stable_diagnostic: String,
    pub results: Vec<ExampleProjectionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleProjectionResult {
    pub source: String,
    pub target: String,
    pub binding: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleStudy {
    pub kind: String,
    pub compatibility: String,
    pub solver: Option<String>,
    pub results: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<ExampleStudyStage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compatibility_aggregates: Vec<ExampleCompatibilityAggregate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleStudyStage {
    pub index: u32,
    pub id: String,
    pub kind: String,
    pub compatibility: String,
    pub results: Vec<String>,
    pub input_state: String,
    pub output_state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleCompatibilityAggregate {
    pub name: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleSource {
    pub entry: String,
    pub root: String,
    pub tree_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleManifestFile {
    pub path: String,
    pub role: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ExampleSourceFile {
    pub virtual_path: String,
    pub source_path: String,
    pub sha256: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
struct RawManifest {
    schema_version: u32,
    artifact: String,
    generator: String,
    psa_contract_version: String,
    entry: String,
    study: RawStudy,
    source: RawSource,
    projection: RawProjection,
    files: Vec<RawFile>,
    #[serde(default = "empty_inputs")]
    inputs: Value,
}

#[derive(Debug, Deserialize)]
struct RawProjection {
    source_function: String,
    profile: String,
    projector: String,
    target_function: String,
    managed_entry: String,
    status: String,
    stable_diagnostic: String,
    results: Vec<RawProjectionResult>,
}

#[derive(Debug, Deserialize)]
struct RawProjectionResult {
    source: String,
    target: String,
    binding: String,
}

#[derive(Debug, Deserialize)]
struct RawStudy {
    kind: String,
    compatibility: String,
    #[serde(default)]
    solver: Option<String>,
    #[serde(default)]
    results: Vec<String>,
    #[serde(default)]
    stages: Vec<RawStudyStage>,
    #[serde(default)]
    compatibility_aggregates: Vec<RawCompatibilityAggregate>,
}

#[derive(Debug, Deserialize)]
struct RawStudyStage {
    index: u32,
    id: String,
    kind: String,
    compatibility: String,
    #[serde(default)]
    results: Vec<String>,
    input_state: String,
    output_state: String,
}

#[derive(Debug, Deserialize)]
struct RawCompatibilityAggregate {
    name: String,
    sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawSource {
    entry: String,
    root: String,
    tree_sha256: String,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    path: String,
    role: String,
    sha256: String,
}

fn empty_inputs() -> Value {
    Value::Array(Vec::new())
}

impl From<RawManifest> for ExampleManifest {
    fn from(raw: RawManifest) -> Self {
        Self {
            schema_version: raw.schema_version,
            artifact: raw.artifact,
            generator: raw.generator,
            psa_contract_version: raw.psa_contract_version,
            entry: raw.entry,
            study: ExampleStudy {
                kind: raw.study.kind,
                compatibility: raw.study.compatibility,
                solver: raw.study.solver,
                results: raw.study.results,
                stages: raw
                    .study
                    .stages
                    .into_iter()
                    .map(|stage| ExampleStudyStage {
                        index: stage.index,
                        id: stage.id,
                        kind: stage.kind,
                        compatibility: stage.compatibility,
                        results: stage.results,
                        input_state: stage.input_state,
                        output_state: stage.output_state,
                    })
                    .collect(),
                compatibility_aggregates: raw
                    .study
                    .compatibility_aggregates
                    .into_iter()
                    .map(|aggregate| ExampleCompatibilityAggregate {
                        name: aggregate.name,
                        sources: aggregate.sources,
                    })
                    .collect(),
            },
            source: ExampleSource {
                entry: raw.source.entry,
                root: raw.source.root,
                tree_sha256: raw.source.tree_sha256,
            },
            projection: ExampleProjection {
                source_function: raw.projection.source_function,
                profile: raw.projection.profile,
                projector: raw.projection.projector,
                target_function: raw.projection.target_function,
                managed_entry: raw.projection.managed_entry,
                status: raw.projection.status,
                stable_diagnostic: raw.projection.stable_diagnostic,
                results: raw
                    .projection
                    .results
                    .into_iter()
                    .map(|result| ExampleProjectionResult {
                        source: result.source,
                        target: result.target,
                        binding: result.binding,
                    })
                    .collect(),
            },
            files: raw
                .files
                .into_iter()
                .map(|file| ExampleManifestFile {
                    path: file.path,
                    role: file.role,
                    sha256: file.sha256,
                })
                .collect(),
            inputs: raw.inputs,
        }
    }
}

pub(crate) fn load_examples(
    engine_root: &Path,
    dss_root: &Path,
    bundle_specs: &[SourceFileSpec],
    projections: &CaseProjectionRegistry,
    limits: ResourceLimits,
) -> Result<Vec<EngineExample>, EngineResourceError> {
    let mut examples = Vec::with_capacity(bundle_specs.len());
    let mut ids = HashSet::new();
    for spec in bundle_specs {
        let manifest_path = safe_file(dss_root, &spec.path)?;
        let bytes = read_bounded_file(&manifest_path, limits.max_example_bytes, "example bytes")?;
        verify_loaded_hash(&bytes, &spec.sha256)?;
        let bundle_path = spec
            .path
            .strip_prefix("rssbundle/")
            .and_then(|path| path.strip_suffix("/manifest.json"))
            .ok_or_else(|| {
                EngineResourceError::new("example_path_invalid", "bundle manifest path")
            })?
            .to_string();
        let id = example_id("rssbundle", &bundle_path);
        if !ids.insert(id.clone()) {
            return Err(EngineResourceError::new(
                "duplicate_example_id",
                &bundle_path,
            ));
        }
        let manifest_source = manifest_path
            .strip_prefix(engine_root)
            .ok()
            .and_then(Path::to_str)
            .map(|path| path.replace('\\', "/"))
            .ok_or_else(|| EngineResourceError::new("example_path_invalid", "manifest source"))?;
        let raw: RawManifest = match serde_json::from_slice(&bytes) {
            Ok(raw) => raw,
            Err(_) => {
                examples.push(invalid_example(
                    &bundle_path,
                    id,
                    manifest_source,
                    &bytes,
                    "manifest_invalid",
                ));
                continue;
            }
        };
        if !SUPPORTED_BUNDLE_SCHEMA_VERSIONS.contains(&raw.schema_version) {
            examples.push(invalid_example(
                &bundle_path,
                id,
                manifest_source,
                &bytes,
                "manifest_schema_unsupported",
            ));
            continue;
        }
        if !valid_bundle_schema(&raw) {
            examples.push(invalid_example(
                &bundle_path,
                id,
                manifest_source,
                &bytes,
                "manifest_schema_invalid",
            ));
            continue;
        }
        if raw.artifact != SUPPORTED_BUNDLE_ARTIFACT {
            examples.push(invalid_example(
                &bundle_path,
                id,
                manifest_source,
                &bytes,
                "manifest_artifact_unsupported",
            ));
            continue;
        }
        if !safe_relative(&raw.entry)
            || !safe_relative(&raw.source.entry)
            || !safe_relative(&raw.source.root)
            || !safe_relative(&raw.projection.managed_entry)
            || raw.files.iter().any(|file| !safe_relative(&file.path))
        {
            examples.push(invalid_example(
                &bundle_path,
                id,
                manifest_source,
                &bytes,
                "manifest_path_unsafe: [redacted]",
            ));
            continue;
        }
        if raw.files.len().saturating_add(1) > limits.max_example_files {
            return Err(limit_error("example files"));
        }
        let manifest: ExampleManifest = raw.into();
        let bundle_root = manifest_path
            .parent()
            .ok_or_else(|| EngineResourceError::new("example_path_invalid", "bundle root"))?;
        let mut diagnostics = Vec::new();
        let entry_safe = true;
        let mut declared = HashSet::new();
        let mut total = bytes.len() as u64;
        let mut tree = Sha256::new();
        tree.update(spec.path.as_bytes());
        tree.update(&bytes);
        let mut files = vec![ExampleSourceFile {
            virtual_path: "manifest.json".to_string(),
            source_path: manifest_source,
            sha256: spec.sha256.clone(),
            role: "manifest".to_string(),
        }];
        let mut entry_found = false;
        let mut source_entry_found = false;
        let mut managed_entry_found = false;
        for declared_file in &manifest.files {
            if !declared.insert(declared_file.path.clone()) {
                diagnostics.push(format!("file_path_duplicate: {}", declared_file.path));
                continue;
            }
            if entry_safe
                && declared_file.path == manifest.entry
                && declared_file.role == "main-rss"
            {
                entry_found = true;
            }
            if declared_file.path == manifest.source.entry && declared_file.role == "dss-entry" {
                source_entry_found = true;
            }
            if declared_file.path == manifest.projection.managed_entry
                && declared_file.role == "managed-rss"
            {
                managed_entry_found = true;
            }
            let path = match safe_file(bundle_root, &declared_file.path) {
                Ok(path) => path,
                Err(_) => {
                    diagnostics.push(format!("file_missing_or_unsafe: {}", declared_file.path));
                    continue;
                }
            };
            let remaining = limits
                .max_example_bytes
                .checked_sub(total)
                .ok_or_else(|| limit_error("example bytes"))?;
            let (actual_hash, size) =
                match hash_bounded_file_with_size(&path, remaining, "example bytes") {
                    Ok(result) => result,
                    Err(_) => {
                        diagnostics.push(format!("file_missing_or_unsafe: {}", declared_file.path));
                        continue;
                    }
                };
            total = total.saturating_add(size);
            tree.update(declared_file.path.as_bytes());
            tree.update(actual_hash.as_bytes());
            if actual_hash != declared_file.sha256 {
                diagnostics.push(format!("file_hash_mismatch: {}", declared_file.path));
            }
            let source_path = path
                .strip_prefix(engine_root)
                .ok()
                .and_then(Path::to_str)
                .map(|path| path.replace('\\', "/"))
                .ok_or_else(|| EngineResourceError::new("example_path_invalid", "file source"))?;
            files.push(ExampleSourceFile {
                virtual_path: declared_file.path.clone(),
                source_path,
                sha256: actual_hash,
                role: declared_file.role.clone(),
            });
        }
        if !entry_found {
            diagnostics.push("entry_not_declared".to_string());
        }
        if !managed_entry_found {
            diagnostics.push("managed_entry_not_declared".to_string());
        }
        if !source_entry_found
            || !Path::new(&manifest.source.entry).starts_with(&manifest.source.root)
        {
            diagnostics.push("model_import_source_missing".to_string());
        }
        validate_projection(&manifest, projections, &mut diagnostics);
        let category_path = category_path(&bundle_path)?;
        examples.push(EngineExample {
            id,
            title: case_title(&bundle_path),
            source_kind: "rssbundle".to_string(),
            category_path,
            bundle_path,
            entry_path: entry_safe.then(|| manifest.entry.clone()),
            manifest: Some(manifest),
            file_count: files.len(),
            size_bytes: total,
            tree_hash: hex::encode(tree.finalize()),
            importable: true,
            diagnostics,
            files,
        });
    }
    examples.sort_by(|left, right| left.bundle_path.cmp(&right.bundle_path));
    Ok(examples)
}

fn validate_projection(
    manifest: &ExampleManifest,
    projections: &CaseProjectionRegistry,
    diagnostics: &mut Vec<String>,
) {
    let profile = match projections.get_enabled(&manifest.projection.profile) {
        Some(profile) => profile,
        None => {
            let code = projections
                .diagnostic(&manifest.projection.profile)
                .map(|diagnostic| diagnostic.code.as_str())
                .unwrap_or("case_projection_unknown");
            diagnostics.push(code.to_string());
            return;
        }
    };
    if !projection_matches(manifest, profile) {
        diagnostics.push(format!(
            "{}: projection_contract_mismatch",
            profile.stable_diagnostic
        ));
    }
}

fn projection_matches(manifest: &ExampleManifest, profile: &CaseProjectionProfile) -> bool {
    let projection = &manifest.projection;
    if projection.source_function != profile.source_function
        || projection.projector != profile.projector
        || projection.target_function != profile.target_function
        || projection.managed_entry != profile.managed_entry
        || projection.status != "enabled"
        || projection.stable_diagnostic != profile.stable_diagnostic
        || manifest.study.results.is_empty()
        || projection.results.is_empty()
    {
        return false;
    }
    if manifest.schema_version == 3 {
        return projection_matches_sequence(manifest, profile);
    }
    let mut native_sources = HashSet::new();
    if !manifest.study.results.iter().all(|source| {
        native_sources.insert(source.as_str()) && profile.bundle_result_map.contains_key(source)
    }) {
        return false;
    }
    let mut managed_sources = HashSet::new();
    let mut remaining_native = manifest.study.results.iter();
    projection.results.iter().all(|result| {
        if !managed_sources.insert(result.source.as_str()) {
            return false;
        }
        let Some(source) = remaining_native.find(|source| **source == result.source) else {
            return false;
        };
        profile.bundle_result_map.get(source) == Some(&result.target)
            && result.binding == format!("result_{source}")
    })
}

fn projection_matches_sequence(
    manifest: &ExampleManifest,
    profile: &CaseProjectionProfile,
) -> bool {
    if profile.id != "dss_study_sequence_v1"
        || manifest.projection.results.len() != manifest.study.results.len()
    {
        return false;
    }
    manifest.projection.results.iter().all(|result| {
        sequence_target_name(manifest, &result.source).is_some_and(|name| name == result.target)
    })
}

fn valid_bundle_schema(raw: &RawManifest) -> bool {
    if raw.schema_version == 2 {
        return raw.study.kind != "study_sequence"
            && raw.projection.profile != "dss_study_sequence_v1"
            && raw.study.stages.is_empty()
            && raw.study.compatibility_aggregates.is_empty();
    }
    if raw.schema_version != 3
        || raw.study.kind != "study_sequence"
        || raw.study.compatibility != "opendss-native"
        || raw.projection.profile != "dss_study_sequence_v1"
        || raw.study.results.is_empty()
        || !unique_identifiers(&raw.study.results)
        || raw.study.stages.is_empty()
    {
        return false;
    }
    let mut stage_ids = HashSet::new();
    for (index, stage) in raw.study.stages.iter().enumerate() {
        if stage.index as usize != index
            || !safe_identifier(&stage.id)
            || !stage_ids.insert(stage.id.as_str())
            || !matches!(
                stage.kind.as_str(),
                "snapshot" | "direct" | "fault_study" | "harmonic"
            )
            || stage.compatibility != "opendss-native"
            || !unique_identifiers(&stage.results)
            || stage.input_state.is_empty()
            || stage.output_state.is_empty()
        {
            return false;
        }
    }
    let mut aggregate_names = HashSet::new();
    for aggregate in &raw.study.compatibility_aggregates {
        if !safe_identifier(&aggregate.name)
            || !aggregate_names.insert(aggregate.name.as_str())
            || aggregate.sources.is_empty()
            || aggregate
                .sources
                .iter()
                .any(|source| !valid_stage_source(&raw.study.stages, source))
        {
            return false;
        }
    }
    if raw.projection.results.len() != raw.study.results.len() {
        return false;
    }
    let mut sources = HashSet::new();
    let mut bindings = HashSet::new();
    raw.study
        .results
        .iter()
        .zip(&raw.projection.results)
        .all(|(public, result)| {
            safe_identifier(&result.binding)
                && bindings.insert(result.binding.as_str())
                && !result.source.is_empty()
                && sources.insert(result.source.as_str())
                && if result.source == "study_status" {
                    result.target == "study_status" && result.binding == "study_status"
                } else if aggregate_names.contains(result.source.as_str()) {
                    result.source == *public
                        && raw_aggregate_target(&raw.study, &result.source)
                            .is_some_and(|target| target == result.target)
                } else {
                    valid_stage_source(&raw.study.stages, &result.source)
                        && stage_source_result(&result.source)
                            .is_some_and(|name| name == result.target)
                }
        })
}

fn unique_identifiers(values: &[String]) -> bool {
    let mut unique = HashSet::new();
    values
        .iter()
        .all(|value| safe_identifier(value) && unique.insert(value.as_str()))
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn stage_source_result(source: &str) -> Option<&str> {
    let (index, result) = source.split_once('.')?;
    index.parse::<usize>().ok()?;
    safe_identifier(result).then_some(result)
}

fn valid_stage_source(stages: &[RawStudyStage], source: &str) -> bool {
    let Some((index, result)) = source.split_once('.') else {
        return false;
    };
    index
        .parse::<usize>()
        .ok()
        .and_then(|index| stages.get(index))
        .is_some_and(|stage| stage.results.iter().any(|candidate| candidate == result))
}

fn sequence_target_name<'a>(manifest: &'a ExampleManifest, source: &'a str) -> Option<&'a str> {
    if source == "study_status" {
        return Some("study_status");
    }
    if let Some(aggregate) = manifest
        .study
        .compatibility_aggregates
        .iter()
        .find(|aggregate| aggregate.name == source)
    {
        return aggregate_target(&aggregate.sources);
    }
    let (index, result) = source.split_once('.')?;
    let stage = manifest.study.stages.get(index.parse::<usize>().ok()?)?;
    stage
        .results
        .iter()
        .any(|name| name == result)
        .then_some(result)
}

fn raw_aggregate_target<'a>(study: &'a RawStudy, name: &str) -> Option<&'a str> {
    let aggregate = study
        .compatibility_aggregates
        .iter()
        .find(|aggregate| aggregate.name == name)?;
    aggregate_target(&aggregate.sources)
}

fn aggregate_target(sources: &[String]) -> Option<&str> {
    let mut sources = sources.iter();
    let target = stage_source_result(sources.next()?)?;
    sources
        .all(|source| stage_source_result(source) == Some(target))
        .then_some(target)
}

fn invalid_example(
    bundle_path: &str,
    id: String,
    manifest_source: String,
    bytes: &[u8],
    diagnostic: &str,
) -> EngineExample {
    let mut tree = Sha256::new();
    tree.update(format!("{bundle_path}/manifest.json").as_bytes());
    tree.update(bytes);
    EngineExample {
        id,
        title: case_title(bundle_path),
        source_kind: "rssbundle".to_string(),
        category_path: category_path(bundle_path).unwrap_or_default(),
        bundle_path: bundle_path.to_string(),
        entry_path: None,
        manifest: None,
        file_count: 1,
        size_bytes: bytes.len() as u64,
        tree_hash: hex::encode(tree.finalize()),
        importable: false,
        diagnostics: vec![diagnostic.to_string()],
        files: vec![ExampleSourceFile {
            virtual_path: "manifest.json".to_string(),
            source_path: manifest_source,
            sha256: hex::encode(Sha256::digest(bytes)),
            role: "manifest".to_string(),
        }],
    }
}

fn category_path(bundle_path: &str) -> Result<Vec<String>, EngineResourceError> {
    let parent = Path::new(bundle_path)
        .parent()
        .ok_or_else(|| EngineResourceError::new("example_path_invalid", "bundle category"))?;
    parent
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| EngineResourceError::new("example_path_invalid", "category utf8")),
            _ => Err(EngineResourceError::new(
                "example_path_invalid",
                "bundle category",
            )),
        })
        .collect()
}

fn case_title(bundle_path: &str) -> String {
    Path::new(bundle_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("RSS bundle")
        .to_string()
}

fn safe_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn verify_loaded_hash(bytes: &[u8], expected: &str) -> Result<(), EngineResourceError> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(EngineResourceError::new(
            "engine_resource_changed_during_scan",
            "resource changed while reading",
        ))
    }
}

fn example_id(kind: &str, path: &str) -> String {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let hash = hex::encode(Sha256::digest(format!("{kind}:{normalized}").as_bytes()));
    format!("ex_{}", &hash[..26])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence_manifest() -> RawManifest {
        RawManifest {
            schema_version: 3,
            artifact: SUPPORTED_BUNDLE_ARTIFACT.into(),
            generator: "test".into(),
            psa_contract_version: "test".into(),
            entry: "main.rss".into(),
            study: RawStudy {
                kind: "study_sequence".into(),
                compatibility: "opendss-native".into(),
                solver: None,
                results: vec!["bus_voltage".into()],
                stages: vec![RawStudyStage {
                    index: 0,
                    id: "snapshot".into(),
                    kind: "snapshot".into(),
                    compatibility: "opendss-native".into(),
                    results: vec!["bus_voltage".into()],
                    input_state: "source".into(),
                    output_state: "fundamental".into(),
                }],
                compatibility_aggregates: vec![],
            },
            source: RawSource {
                entry: "dss/model.dss".into(),
                root: "dss".into(),
                tree_sha256: "0".repeat(64),
            },
            projection: RawProjection {
                source_function: "ds_dss_study_sequence".into(),
                profile: "dss_study_sequence_v1".into(),
                projector: "project_dss_study_sequence".into(),
                target_function: "ds_study_sequence".into(),
                managed_entry: "managed.rss".into(),
                status: "enabled".into(),
                stable_diagnostic: "PSA.DSS.PROJECTION.INVALID".into(),
                results: vec![RawProjectionResult {
                    source: "0.bus_voltage".into(),
                    target: "bus_voltage".into(),
                    binding: "bus_voltage".into(),
                }],
            },
            files: vec![],
            inputs: Value::Array(vec![]),
        }
    }

    #[test]
    fn schema_three_accepts_only_valid_ordered_study_sequences() {
        let manifest = sequence_manifest();
        assert!(valid_bundle_schema(&manifest));

        let mut invalid = sequence_manifest();
        invalid.study.stages[0].index = 1;
        assert!(!valid_bundle_schema(&invalid));

        let mut reserved = sequence_manifest();
        reserved.study.kind = "static_power_flow".into();
        assert!(!valid_bundle_schema(&reserved));

        let mut legacy_binding = sequence_manifest();
        legacy_binding.study.results[0] = "voltage".into();
        legacy_binding.projection.results[0].binding = "result_voltage".into();
        assert!(valid_bundle_schema(&legacy_binding));

        let mut aggregate = sequence_manifest();
        aggregate.study.results[0] = "voltage".into();
        aggregate.study.stages.push(RawStudyStage {
            index: 1,
            id: "direct".into(),
            kind: "direct".into(),
            compatibility: "opendss-native".into(),
            results: vec!["bus_voltage".into()],
            input_state: "fundamental".into(),
            output_state: "direct".into(),
        });
        aggregate.study.compatibility_aggregates = vec![RawCompatibilityAggregate {
            name: "voltage".into(),
            sources: vec!["0.bus_voltage".into(), "1.bus_voltage".into()],
        }];
        aggregate.projection.results[0] = RawProjectionResult {
            source: "voltage".into(),
            target: "bus_voltage".into(),
            binding: "voltage".into(),
        };
        assert!(valid_bundle_schema(&aggregate));

        aggregate.study.stages[1].results[0] = "branch_flow".into();
        aggregate.study.compatibility_aggregates[0].sources[1] = "1.branch_flow".into();
        assert!(!valid_bundle_schema(&aggregate));
    }

    #[test]
    fn schema_three_preserves_stage_contract_in_the_published_manifest() {
        let manifest = ExampleManifest::from(sequence_manifest());
        assert_eq!(manifest.study.stages.len(), 1);
        assert_eq!(manifest.study.stages[0].id, "snapshot");
        assert_eq!(manifest.study.stages[0].results, ["bus_voltage"]);
    }
}
