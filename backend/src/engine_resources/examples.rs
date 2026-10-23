use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::snapshot::{ResourceLimits, SourceFileSpec};
use super::{EngineResourceError, read_bounded_file, safe_file};

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
    #[serde(skip)]
    pub files: Vec<ExampleSourceFile>,
}

#[derive(Debug, Clone)]
pub struct ExampleSourceFile {
    pub virtual_path: String,
    pub source_path: String,
    pub sha256: String,
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
    limits: ResourceLimits,
) -> Result<Vec<EngineExample>, EngineResourceError> {
    let mut examples = Vec::new();
    let mut ids = HashSet::new();
    for spec in root_specs {
        let path = safe_file(dss_root, &spec.path)?;
        let bytes = Arc::new(read_bounded_file(
            &path,
            limits.max_example_bytes,
            "example bytes",
        )?);
        verify_loaded_hash(&bytes, &spec.sha256)?;
        let id = example_id("rss", &spec.path);
        if !ids.insert(id.clone()) {
            return Err(EngineResourceError::new(
                "duplicate_example_id",
                spec.path.clone(),
            ));
        }
        let mut diagnostics = Vec::new();
        if contains_dynamic_dependency(&bytes) {
            diagnostics.push("dynamic dependency".to_string());
        }
        let dependencies = discover_rss_dependencies(
            engine_root,
            &path,
            Arc::clone(&bytes),
            &mut diagnostics,
            limits.max_dependency_depth,
            limits.max_example_files,
            limits.max_example_bytes,
        )?;
        let entry_source = format!("sparrowzz/dss/{}", spec.path);
        let mut files = vec![ExampleSourceFile {
            virtual_path: entry_source.clone(),
            source_path: entry_source,
            sha256: spec.sha256.clone(),
        }];
        let mut total = bytes.len() as u64;
        let mut tree = Sha256::new();
        tree.update(spec.path.as_bytes());
        tree.update(bytes.as_slice());
        for (dependency, data) in &dependencies {
            total += data.len() as u64;
            let relative = dependency
                .strip_prefix(engine_root)
                .map_err(|_| {
                    EngineResourceError::new(
                        "example_path_invalid",
                        dependency.display().to_string(),
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            tree.update(relative.as_bytes());
            tree.update(data.as_slice());
            files.push(ExampleSourceFile {
                virtual_path: relative.clone(),
                source_path: relative,
                sha256: format!("{:x}", Sha256::digest(data.as_slice())),
            });
        }
        if files.len() > limits.max_example_files || total > limits.max_example_bytes {
            return Err(super::limit_error("example files or bytes"));
        }
        files.sort_by(|left, right| left.virtual_path.cmp(&right.virtual_path));
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
            files,
        });
    }
    for spec in bundle_specs {
        let manifest_path = safe_file(dss_root, &spec.path)?;
        let bytes = read_bounded_file(&manifest_path, limits.max_example_bytes, "example bytes")?;
        verify_loaded_hash(&bytes, &spec.sha256)?;
        let rel_bundle = spec.path.trim_end_matches("/manifest.json");
        let id = example_id("rssbundle", rel_bundle);
        if !ids.insert(id.clone()) {
            return Err(EngineResourceError::new("duplicate_example_id", rel_bundle));
        }
        let bundle_root = manifest_path
            .parent()
            .ok_or_else(|| EngineResourceError::new("example_path_invalid", &spec.path))?;
        let manifest_source = manifest_path
            .strip_prefix(engine_root)
            .map_err(|_| {
                EngineResourceError::new(
                    "example_path_invalid",
                    manifest_path.display().to_string(),
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");
        let manifest: BundleManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => {
                examples.push(invalid_bundle_example(
                    rel_bundle,
                    id,
                    manifest_source,
                    &bytes,
                    "invalid manifest",
                ));
                continue;
            }
        };
        if manifest.files.len().saturating_add(1) > limits.max_example_files {
            return Err(super::limit_error("example files"));
        }
        let entry_safe = safe_relative(&manifest.entry).is_ok();
        let mut diagnostics = Vec::new();
        if !entry_safe {
            diagnostics.push("unsafe entry: [redacted]".to_string());
        }
        let mut total = bytes.len() as u64;
        let mut tree = Sha256::new();
        tree.update(spec.path.as_bytes());
        tree.update(&bytes);
        let mut entry_found = false;
        let mut files = vec![ExampleSourceFile {
            virtual_path: "manifest.json".to_string(),
            source_path: manifest_source,
            sha256: spec.sha256.clone(),
        }];
        for file in &manifest.files {
            if safe_relative(&file.path).is_err() {
                diagnostics.push("unsafe file: [redacted]".to_string());
                continue;
            }
            let path = match safe_file(bundle_root, &file.path) {
                Ok(path) => path,
                Err(_) => {
                    diagnostics.push(format!("missing file: {}", file.path));
                    continue;
                }
            };
            let remaining = limits
                .max_example_bytes
                .checked_sub(total)
                .ok_or_else(|| super::limit_error("example bytes"))?;
            match read_bounded_file(&path, remaining, "example bytes") {
                Ok(data) => {
                    total += data.len() as u64;
                    let hash = format!("{:x}", Sha256::digest(&data));
                    tree.update(file.path.as_bytes());
                    tree.update(hash.as_bytes());
                    if hash != file.sha256 {
                        diagnostics.push(format!("hash mismatch: {}", file.path));
                    }
                    if entry_safe && file.path == manifest.entry {
                        entry_found = true;
                    }
                    let source_path = path
                        .strip_prefix(engine_root)
                        .map_err(|_| {
                            EngineResourceError::new(
                                "example_path_invalid",
                                path.display().to_string(),
                            )
                        })?
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.push(ExampleSourceFile {
                        virtual_path: file.path.clone(),
                        source_path,
                        sha256: hash,
                    });
                }
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
        examples.push(EngineExample {
            id,
            title: Path::new(rel_bundle)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("RSS bundle")
                .to_string(),
            source_kind: "rssbundle".to_string(),
            entry_path: if entry_safe {
                format!("{rel_bundle}/{}", manifest.entry)
            } else {
                format!("{rel_bundle}/[invalid-entry]")
            },
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
            files,
        });
    }
    Ok(examples)
}

fn verify_loaded_hash(bytes: &[u8], expected: &str) -> Result<(), EngineResourceError> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(EngineResourceError::new(
            "engine_resource_changed_during_scan",
            "resource changed while reading",
        ))
    }
}

fn invalid_bundle_example(
    rel_bundle: &str,
    id: String,
    manifest_source: String,
    bytes: &[u8],
    diagnostic: &str,
) -> EngineExample {
    let mut tree = Sha256::new();
    tree.update(format!("{rel_bundle}/manifest.json").as_bytes());
    tree.update(bytes);
    EngineExample {
        id,
        title: Path::new(rel_bundle)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("RSS bundle")
            .to_string(),
        source_kind: "rssbundle".to_string(),
        entry_path: format!("{rel_bundle}/[invalid-entry]"),
        study_type: "unknown".to_string(),
        solver: None,
        required_features: Vec::new(),
        file_count: 1,
        size_bytes: bytes.len() as u64,
        tree_hash: format!("{:x}", tree.finalize()),
        importable: false,
        diagnostics: vec![diagnostic.to_string()],
        files: vec![ExampleSourceFile {
            virtual_path: "manifest.json".to_string(),
            source_path: manifest_source,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }],
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
    entry_path: &Path,
    bytes: Arc<Vec<u8>>,
    diagnostics: &mut Vec<String>,
    max_depth: usize,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<(PathBuf, Arc<Vec<u8>>)>, EngineResourceError> {
    let mut dependencies = Vec::new();
    let mut visited = HashSet::from([entry_path.to_path_buf()]);
    let mut total_bytes = bytes.len() as u64;
    if max_files == 0 || total_bytes > max_bytes {
        return Err(super::limit_error("example files or bytes"));
    }
    let mut queue = vec![(entry_path.to_path_buf(), bytes, 0_usize)];
    while let Some((current, data, depth)) = queue.pop() {
        let text = String::from_utf8_lossy(&data);
        for token in dependency_tokens(&text) {
            if token.contains("://") {
                diagnostics.push("external dependency: [redacted]".to_string());
                continue;
            }
            if safe_relative(&token).is_err() {
                diagnostics.push("unsafe dependency: [redacted]".to_string());
                continue;
            }
            let candidate = if token.starts_with("sparrowzz/") || token.starts_with("psa/") {
                engine_root.join(&token)
            } else {
                current.parent().unwrap_or(engine_root).join(&token)
            };
            match contained_regular_file(engine_root, &candidate) {
                Some(path) => {
                    if visited.insert(path.clone()) {
                        if visited.len() > max_files {
                            return Err(super::limit_error("example files"));
                        }
                        let remaining = max_bytes
                            .checked_sub(total_bytes)
                            .ok_or_else(|| super::limit_error("example bytes"))?;
                        match read_bounded_file(&path, remaining, "example bytes") {
                            Ok(bytes) => {
                                total_bytes += bytes.len() as u64;
                                let bytes = Arc::new(bytes);
                                dependencies.push((path.clone(), Arc::clone(&bytes)));
                                if path
                                    .extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| {
                                        extension.eq_ignore_ascii_case("rss")
                                            || extension.eq_ignore_ascii_case("dss")
                                    })
                                {
                                    let next_depth = depth.saturating_add(1);
                                    if next_depth > max_depth {
                                        return Err(super::limit_error("dependency depth"));
                                    }
                                    queue.push((path, bytes, next_depth));
                                }
                            }
                            Err(_) => diagnostics.push(format!("missing dependency: {token}")),
                        }
                    }
                }
                _ => diagnostics.push(format!("missing dependency: {token}")),
            }
        }
    }
    dependencies.sort();
    Ok(dependencies)
}

fn contained_regular_file(engine_root: &Path, candidate: &Path) -> Option<PathBuf> {
    for ancestor in candidate.ancestors() {
        if ancestor == engine_root {
            break;
        }
        if std::fs::symlink_metadata(ancestor)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return None;
        }
    }
    candidate
        .canonicalize()
        .ok()
        .filter(|path| path.starts_with(engine_root) && path.is_file())
}

fn contains_dynamic_dependency(bytes: &[u8]) -> bool {
    let source = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    ["import_dss(", "import_bmopf(", "read("]
        .iter()
        .any(|call| {
            source.match_indices(call).any(|(index, _)| {
                !source[index + call.len()..]
                    .trim_start()
                    .starts_with(['\"', '\''])
            })
        })
}

fn dependency_tokens(text: &str) -> Vec<String> {
    let executable = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && !line.starts_with('!')
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut tokens: Vec<String> = executable
        .split(['"', '\''])
        .enumerate()
        .filter_map(|(index, token)| {
            (index % 2 == 1 && is_dependency(token)).then(|| token.to_string())
        })
        .collect();
    for line in executable.lines() {
        let mut words = line.split_whitespace();
        if words.next().is_some_and(|word| {
            word.eq_ignore_ascii_case("redirect") || word.eq_ignore_ascii_case("compile")
        }) {
            if let Some(path) = words
                .next()
                .map(|word| word.trim_matches(['"', '\'', '(', ')']))
            {
                if is_dependency(path) {
                    tokens.push(path.to_string());
                }
            }
        }
        for word in line.split_whitespace() {
            let path = word.trim_matches(['"', '\'', '(', ')', ',', ';']);
            if !path.contains('(') && !path.contains('=') && is_dependency(path) {
                tokens.push(path.to_string());
            }
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

fn is_dependency(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    [".dss", ".csv", ".json", ".rss"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
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
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{kind}:{normalized}").as_bytes())
    );
    format!("ex_{}", &hash[..26])
}
