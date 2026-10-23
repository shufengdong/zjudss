mod examples;
mod knowledge;
mod snapshot;

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::adapters::ENGINE_CONTRACT_VERSION;

pub use examples::{EngineExample, ExampleSourceFile};
pub use knowledge::{DocSearchHit, DocSection, EngineKnowledge};
use snapshot::SourceFileSpec;
pub use snapshot::{EngineResourceSnapshot, ResourceLimits};

pub const SUPPORTED_CATALOG_SCHEMA_VERSIONS: &[u32] = &[2];

#[derive(Debug)]
pub struct EngineResourceError {
    code: &'static str,
    message: String,
}

impl EngineResourceError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        Self::new("engine_resource_io", error.to_string())
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for EngineResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineResourceError {}

pub struct EngineResourceRegistry;

impl EngineResourceRegistry {
    pub fn load(
        tensoreval_cwd: &Path,
        capabilities: &HashSet<String>,
    ) -> Result<EngineResourceSnapshot, EngineResourceError> {
        Self::load_with_limits(tensoreval_cwd, capabilities, ResourceLimits::default())
    }

    pub fn load_with_limits(
        tensoreval_cwd: &Path,
        capabilities: &HashSet<String>,
        limits: ResourceLimits,
    ) -> Result<EngineResourceSnapshot, EngineResourceError> {
        let engine_root = tensoreval_cwd
            .canonicalize()
            .map_err(EngineResourceError::io)?;
        let dss_root = safe_directory(&engine_root, "sparrowzz/dss")?;
        let dataframes = read_safe(&dss_root, "docs/catalog/dataframes.json")?;
        let functions = read_safe(&dss_root, "docs/catalog/functions.json")?;
        let catalog_json: serde_json::Value = serde_json::from_slice(&dataframes)
            .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
        let functions_json: serde_json::Value = serde_json::from_slice(&functions)
            .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
        let catalog_schema_version = catalog_schema(&catalog_json)?;
        if catalog_schema(&functions_json)? != catalog_schema_version {
            return Err(EngineResourceError::new(
                "engine_contract_mismatch",
                "catalog schema version 彼此不匹配",
            ));
        }
        validate_catalog_shape(&catalog_json, &functions_json)?;

        let mut docs = Vec::new();
        let mut documents = BTreeMap::new();
        let mut total_document_bytes = 0_u64;
        for path in discover_documents(&dss_root)? {
            let relative = portable_relative(&dss_root, &path)?;
            let bytes = std::fs::read(&path).map_err(EngineResourceError::io)?;
            if documents.len() >= limits.max_documents
                || bytes.len() as u64 > limits.max_document_bytes
            {
                return Err(limit_error("documentation count or file size"));
            }
            total_document_bytes = total_document_bytes.saturating_add(bytes.len() as u64);
            if total_document_bytes > limits.max_total_document_bytes {
                return Err(limit_error("documentation total bytes"));
            }
            let content = String::from_utf8(bytes).map_err(|error| {
                EngineResourceError::new("engine_doc_invalid_utf8", error.to_string())
            })?;
            documents.insert(relative.clone(), content.as_bytes().to_vec());
            docs.push((relative.clone(), content));
        }
        let root_rss = discover_root_rss(&dss_root)?;
        let bundles = discover_bundle_manifests(&dss_root)?;
        if root_rss.len().saturating_add(bundles.len()) > limits.max_examples {
            return Err(limit_error("example count"));
        }
        let examples = examples::load_examples(
            &engine_root,
            &dss_root,
            &root_rss,
            &bundles,
            capabilities,
            limits,
        )?;
        let knowledge = EngineKnowledge::from_documents(&docs);
        let mut source_hashes = BTreeMap::from([
            (
                "sparrowzz/dss/docs/catalog/dataframes.json".to_string(),
                hash_bytes(&dataframes),
            ),
            (
                "sparrowzz/dss/docs/catalog/functions.json".to_string(),
                hash_bytes(&functions),
            ),
        ]);
        for (path, bytes) in &documents {
            source_hashes.insert(format!("sparrowzz/dss/{path}"), hash_bytes(bytes));
        }
        for example in &examples {
            for file in &example.files {
                source_hashes.insert(file.source_path.clone(), file.sha256.clone());
            }
        }
        let mut identity = Sha256::new();
        identity_part(&mut identity, b"format", b"zjudss-engine-resource-v1");
        identity_part(
            &mut identity,
            b"contract",
            ENGINE_CONTRACT_VERSION.as_bytes(),
        );
        for (path, hash) in &source_hashes {
            identity_part(&mut identity, path.as_bytes(), hash.as_bytes());
        }
        for example in &examples {
            identity_part(&mut identity, b"example-id", example.id.as_bytes());
            identity_part(&mut identity, b"example-tree", example.tree_hash.as_bytes());
            identity_part(
                &mut identity,
                b"example-importable",
                if example.importable { b"1" } else { b"0" },
            );
            for diagnostic in &example.diagnostics {
                identity_part(&mut identity, b"diagnostic", diagnostic.as_bytes());
            }
        }
        let id = format!("ers_{}", &format!("{:x}", identity.finalize())[..32]);
        Ok(EngineResourceSnapshot {
            id,
            contract_version: ENGINE_CONTRACT_VERSION.to_string(),
            catalog_schema_version,
            source_root: dss_root,
            engine_root,
            catalog_json,
            functions_json,
            catalog_bytes: BTreeMap::from([
                ("dataframes.json".to_string(), dataframes),
                ("functions.json".to_string(), functions),
            ]),
            documents,
            knowledge,
            examples,
            source_hashes,
        })
    }

    pub fn publish(
        snapshot: &EngineResourceSnapshot,
        data_dir: &Path,
    ) -> Result<PathBuf, EngineResourceError> {
        let parent = data_dir.join("system/engine-resources");
        std::fs::create_dir_all(&parent).map_err(EngineResourceError::io)?;
        let destination = parent.join(&snapshot.id);
        if destination.is_dir() {
            return Ok(destination);
        }
        let staging = parent.join(format!(".{}-{}", snapshot.id, std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(EngineResourceError::io)?;
        }
        std::fs::create_dir_all(staging.join("catalog")).map_err(EngineResourceError::io)?;
        for (name, bytes) in &snapshot.catalog_bytes {
            std::fs::write(staging.join("catalog").join(name), bytes)
                .map_err(EngineResourceError::io)?;
        }
        for (relative, bytes) in &snapshot.documents {
            let relative = relative.strip_prefix("docs/").unwrap_or(relative);
            let path = staging.join("docs").join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(EngineResourceError::io)?;
            }
            std::fs::write(path, bytes).map_err(EngineResourceError::io)?;
        }
        std::fs::write(
            staging.join("docs-index.json"),
            serde_json::to_vec_pretty(&snapshot.knowledge.sections).unwrap(),
        )
        .map_err(EngineResourceError::io)?;
        std::fs::write(
            staging.join("examples.json"),
            serde_json::to_vec_pretty(&snapshot.examples).unwrap(),
        )
        .map_err(EngineResourceError::io)?;
        std::fs::write(staging.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "snapshotId": snapshot.id, "engineContractVersion": snapshot.contract_version, "catalogSchemaVersion": snapshot.catalog_schema_version
        })).unwrap()).map_err(EngineResourceError::io)?;
        std::fs::rename(&staging, &destination).map_err(EngineResourceError::io)?;
        Ok(destination)
    }
}

pub(crate) fn hash_file(path: &Path) -> Result<String, EngineResourceError> {
    let bytes = std::fs::read(path).map_err(EngineResourceError::io)?;
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn identity_part(hasher: &mut Sha256, key: &[u8], value: &[u8]) {
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn limit_error(kind: &str) -> EngineResourceError {
    EngineResourceError::new(
        "engine_resource_limit_exceeded",
        format!("engine resource exceeds {kind} limit"),
    )
}

fn read_safe(root: &Path, relative: &str) -> Result<Vec<u8>, EngineResourceError> {
    std::fs::read(safe_file(root, relative)?).map_err(EngineResourceError::io)
}

fn catalog_schema(value: &serde_json::Value) -> Result<u32, EngineResourceError> {
    let schema = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| EngineResourceError::new("catalog_invalid", "缺少 schema_version"))?;
    if !SUPPORTED_CATALOG_SCHEMA_VERSIONS.contains(&schema) {
        return Err(EngineResourceError::new(
            "engine_contract_mismatch",
            format!("不支持 catalog schema version {schema}"),
        ));
    }
    Ok(schema)
}

fn validate_catalog_shape(
    dataframes: &serde_json::Value,
    functions: &serde_json::Value,
) -> Result<(), EngineResourceError> {
    let frames = dataframes
        .get("frames")
        .and_then(serde_json::Value::as_object);
    let analysis = dataframes
        .get("analysis_inputs")
        .and_then(serde_json::Value::as_object);
    if frames.is_none_or(serde_json::Map::is_empty)
        && analysis.is_none_or(serde_json::Map::is_empty)
    {
        return Err(EngineResourceError::new(
            "catalog_invalid",
            "dataframes catalog 缺少 frames/analysis_inputs",
        ));
    }
    if functions
        .get("frame_functions")
        .and_then(serde_json::Value::as_array)
        .is_none()
    {
        return Err(EngineResourceError::new(
            "catalog_invalid",
            "functions catalog 缺少 frame_functions",
        ));
    }
    Ok(())
}

fn portable_relative(root: &Path, path: &Path) -> Result<String, EngineResourceError> {
    path.strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| EngineResourceError::new("engine_resource_path_escape", "路径不在资源根内"))
}

fn discover_documents(dss_root: &Path) -> Result<Vec<PathBuf>, EngineResourceError> {
    let docs_root = safe_directory(dss_root, "docs")?;
    let mut files = Vec::new();
    walk_regular_files(&docs_root, true, &mut files)?;
    files.retain(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    });
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    Ok(files)
}

fn discover_root_rss(dss_root: &Path) -> Result<Vec<SourceFileSpec>, EngineResourceError> {
    let mut paths = std::fs::read_dir(dss_root)
        .map_err(EngineResourceError::io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineResourceError::io)?;
    paths.sort_by_key(std::fs::DirEntry::file_name);
    let mut result = Vec::new();
    for entry in paths {
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(EngineResourceError::io)?;
        if metadata.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("rss"))
        {
            let path = entry.path();
            result.push(SourceFileSpec {
                path: portable_relative(dss_root, &path)?,
                sha256: hash_file(&path)?,
            });
        }
    }
    Ok(result)
}

fn discover_bundle_manifests(dss_root: &Path) -> Result<Vec<SourceFileSpec>, EngineResourceError> {
    let root = dss_root.join("rssbundle");
    if !root.exists() {
        return Ok(Vec::new());
    }
    if std::fs::symlink_metadata(&root)
        .map_err(EngineResourceError::io)?
        .file_type()
        .is_symlink()
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            "rssbundle",
        ));
    }
    let mut files = Vec::new();
    walk_regular_files(&root, false, &mut files)?;
    files.retain(|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("manifest.json")
            && path
                .parent()
                .and_then(Path::extension)
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("rssbundle"))
    });
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    files
        .into_iter()
        .map(|path| {
            Ok(SourceFileSpec {
                path: portable_relative(dss_root, &path)?,
                sha256: hash_file(&path)?,
            })
        })
        .collect()
}

fn walk_regular_files(
    root: &Path,
    reject_symlinks: bool,
    files: &mut Vec<PathBuf>,
) -> Result<(), EngineResourceError> {
    let mut entries = std::fs::read_dir(root)
        .map_err(EngineResourceError::io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineResourceError::io)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(EngineResourceError::io)?;
        if metadata.file_type().is_symlink() {
            if reject_symlinks {
                return Err(EngineResourceError::new(
                    "engine_resource_path_escape",
                    "资源树包含符号链接",
                ));
            }
            continue;
        }
        if metadata.is_dir() {
            walk_regular_files(&path, reject_symlinks, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    safe_path(root, relative, false)
}

fn safe_directory(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    safe_path(root, relative, true)
}

fn safe_path(root: &Path, relative: &str, directory: bool) -> Result<PathBuf, EngineResourceError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            relative,
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        if let Component::Normal(part) = component {
            current.push(part);
            let metadata = std::fs::symlink_metadata(&current).map_err(EngineResourceError::io)?;
            if metadata.file_type().is_symlink() {
                return Err(EngineResourceError::new(
                    "engine_resource_path_escape",
                    relative,
                ));
            }
        }
    }
    let canonical = current.canonicalize().map_err(EngineResourceError::io)?;
    if !canonical.starts_with(root)
        || (directory && !canonical.is_dir())
        || (!directory && !canonical.is_file())
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            relative,
        ));
    }
    Ok(canonical)
}
