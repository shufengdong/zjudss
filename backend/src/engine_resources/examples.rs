use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::snapshot::{ResourceLimits, SourceFileSpec};
use super::{EngineResourceError, hash_bounded_file_with_size, read_bounded_file, safe_file};

const SUPPORTED_BUNDLE_SCHEMA_VERSION: u32 = 1;
const SUPPORTED_BUNDLE_ARTIFACT: &str = "mems.dss-rss-bundle";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    pub files: Vec<ExampleManifestFile>,
    pub inputs: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleStudy {
    pub kind: String,
    pub compatibility: String,
    pub solver: Option<String>,
    pub results: Vec<String>,
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
    files: Vec<RawFile>,
    #[serde(default = "empty_inputs")]
    inputs: Value,
}

#[derive(Debug, Deserialize)]
struct RawStudy {
    kind: String,
    compatibility: String,
    #[serde(default)]
    solver: Option<String>,
    #[serde(default)]
    results: Vec<String>,
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
            },
            source: ExampleSource {
                entry: raw.source.entry,
                root: raw.source.root,
                tree_sha256: raw.source.tree_sha256,
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
        if raw.schema_version != SUPPORTED_BUNDLE_SCHEMA_VERSION {
            examples.push(invalid_example(
                &bundle_path,
                id,
                manifest_source,
                &bytes,
                "manifest_schema_unsupported",
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
            return Err(super::limit_error("example files"));
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
        for declared_file in &manifest.files {
            if !declared.insert(declared_file.path.clone()) {
                diagnostics.push(format!("file_path_duplicate: {}", declared_file.path));
                continue;
            }
            if entry_safe && declared_file.path == manifest.entry {
                entry_found = true;
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
                .ok_or_else(|| super::limit_error("example bytes"))?;
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
        if !manifest
            .files
            .iter()
            .any(|file| file.path.to_ascii_lowercase().ends_with(".dss"))
        {
            diagnostics.push("model_import_source_missing".to_string());
        }
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
            importable: diagnostics.is_empty(),
            diagnostics,
            files,
        });
    }
    examples.sort_by(|left, right| left.bundle_path.cmp(&right.bundle_path));
    Ok(examples)
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
