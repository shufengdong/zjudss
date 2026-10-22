use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::snapshot::SourceFileSpec;
use super::{EngineResourceError, hash_file, safe_file};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineExample {
    pub id: String,
    pub title: String,
    pub source_kind: String,
    pub entry_path: String,
    pub study_type: String,
    pub solver: Option<String>,
    pub required_features: Vec<String>,
    pub file_count: usize,
    pub size_bytes: u64,
    pub tree_hash: String,
    pub importable: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BundleManifest {
    entry: String,
    #[serde(default)]
    files: Vec<BundleFile>,
    #[serde(default)]
    required_features: Vec<String>,
    #[serde(default)]
    study: Option<BundleStudy>,
}

#[derive(Debug, Deserialize)]
struct BundleFile {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct BundleStudy {
    kind: Option<String>,
    solver: Option<String>,
}

pub(crate) fn load_examples(
    engine_root: &Path,
    dss_root: &Path,
    root_specs: &[SourceFileSpec],
    bundle_specs: &[SourceFileSpec],
    capabilities: &HashSet<String>,
) -> Result<Vec<EngineExample>, EngineResourceError> {
    let mut examples = Vec::new();
    let mut ids = HashSet::new();
    for spec in root_specs {
        let path = safe_file(dss_root, &spec.path)?;
        verify_source_hash(&path, &spec.sha256)?;
        let bytes = std::fs::read(&path).map_err(EngineResourceError::io)?;
        let id = example_id("rss", &spec.path);
        if !ids.insert(id.clone()) {
            return Err(EngineResourceError::new(
                "duplicate_example_id",
                spec.path.clone(),
            ));
        }
        let mut diagnostics = Vec::new();
        let dependencies =
            discover_rss_dependencies(engine_root, dss_root, &bytes, &mut diagnostics);
        let mut total = bytes.len() as u64;
        let mut tree = Sha256::new();
        tree.update(spec.path.as_bytes());
        tree.update(&bytes);
        for dependency in &dependencies {
            let data = std::fs::read(dependency).map_err(EngineResourceError::io)?;
            total += data.len() as u64;
            tree.update(dependency.to_string_lossy().as_bytes());
            tree.update(&data);
        }
        examples.push(EngineExample {
            id,
            title: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("RSS example")
                .to_string(),
            source_kind: "rss".to_string(),
            entry_path: spec.path.clone(),
            study_type: infer_study(&spec.path),
            solver: None,
            required_features: Vec::new(),
            file_count: 1 + dependencies.len(),
            size_bytes: total,
            tree_hash: format!("{:x}", tree.finalize()),
            importable: diagnostics.is_empty(),
            diagnostics,
        });
    }
    for spec in bundle_specs {
        let manifest_path = safe_file(dss_root, &spec.path)?;
        verify_source_hash(&manifest_path, &spec.sha256)?;
        let bytes = std::fs::read(&manifest_path).map_err(EngineResourceError::io)?;
        let manifest: BundleManifest = serde_json::from_slice(&bytes).map_err(|error| {
            EngineResourceError::new("example_manifest_invalid", error.to_string())
        })?;
        let bundle_root = manifest_path
            .parent()
            .ok_or_else(|| EngineResourceError::new("example_path_invalid", &spec.path))?;
        safe_relative(&manifest.entry)?;
        let mut diagnostics = Vec::new();
        let mut total = bytes.len() as u64;
        let mut tree = Sha256::new();
        tree.update(&bytes);
        let mut entry_found = false;
        for file in &manifest.files {
            safe_relative(&file.path)?;
            let path = safe_file(bundle_root, &file.path)?;
            match hash_file(&path) {
                Ok(hash) if hash == file.sha256 => {
                    total += std::fs::metadata(&path)
                        .map_err(EngineResourceError::io)?
                        .len();
                    tree.update(file.path.as_bytes());
                    tree.update(hash.as_bytes());
                    if file.path == manifest.entry {
                        entry_found = true;
                    }
                }
                Ok(_) => diagnostics.push(format!("hash mismatch: {}", file.path)),
                Err(_) => diagnostics.push(format!("missing file: {}", file.path)),
            }
        }
        if !entry_found {
            diagnostics.push("entry is not present in files".to_string());
        }
        for feature in &manifest.required_features {
            if !capabilities.contains(feature) {
                diagnostics.push(format!("missing feature: {feature}"));
            }
        }
        let rel_bundle = spec.path.trim_end_matches("/manifest.json");
        let id = example_id("rssbundle", rel_bundle);
        if !ids.insert(id.clone()) {
            return Err(EngineResourceError::new("duplicate_example_id", rel_bundle));
        }
        examples.push(EngineExample {
            id,
            title: Path::new(rel_bundle)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("RSS bundle")
                .to_string(),
            source_kind: "rssbundle".to_string(),
            entry_path: format!("{rel_bundle}/{}", manifest.entry),
            study_type: manifest
                .study
                .as_ref()
                .and_then(|study| study.kind.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            solver: manifest.study.and_then(|study| study.solver),
            required_features: manifest.required_features,
            file_count: manifest.files.len() + 1,
            size_bytes: total,
            tree_hash: format!("{:x}", tree.finalize()),
            importable: diagnostics.is_empty(),
            diagnostics,
        });
    }
    Ok(examples)
}

fn verify_source_hash(path: &Path, expected: &str) -> Result<(), EngineResourceError> {
    let actual = hash_file(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            path.display().to_string(),
        ))
    }
}

fn safe_relative(path: &str) -> Result<(), EngineResourceError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        Err(EngineResourceError::new(
            "engine_resource_path_escape",
            path.display().to_string(),
        ))
    } else {
        Ok(())
    }
}

fn discover_rss_dependencies(
    engine_root: &Path,
    dss_root: &Path,
    bytes: &[u8],
    diagnostics: &mut Vec<String>,
) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(bytes);
    let mut dependencies = Vec::new();
    for token in text.split(['"', '\'']) {
        let lower = token.to_ascii_lowercase();
        if !(lower.ends_with(".dss") || lower.ends_with(".csv") || lower.ends_with(".json"))
            || token.contains("://")
        {
            continue;
        }
        let candidate = if token.starts_with("sparrowzz/") || token.starts_with("psa/") {
            engine_root.join(token)
        } else {
            dss_root.join(token)
        };
        match candidate.canonicalize() {
            Ok(path) if path.starts_with(engine_root) && path.is_file() => dependencies.push(path),
            _ => diagnostics.push(format!("missing dependency: {token}")),
        }
    }
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

fn infer_study(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.contains("short_circuit") {
        "short_circuit"
    } else if lower.contains("harmonic") {
        "harmonic"
    } else if lower.contains("qsts") || lower.contains("daily") {
        "qsts"
    } else {
        "power_flow"
    }
    .to_string()
}

fn example_id(kind: &str, path: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(format!("{kind}:{path}").as_bytes()));
    format!("ex_{}", &hash[..26])
}
