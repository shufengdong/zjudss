mod examples;
mod knowledge;
mod snapshot;

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::adapters::ENGINE_CONTRACT_VERSION;

pub use examples::{
    EngineExample, ExampleManifest, ExampleManifestFile, ExampleSource, ExampleSourceFile,
    ExampleStudy,
};
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
        _capabilities: &HashSet<String>,
        limits: ResourceLimits,
    ) -> Result<EngineResourceSnapshot, EngineResourceError> {
        let engine_root = tensoreval_cwd
            .canonicalize()
            .map_err(EngineResourceError::io)?;
        let dss_root = safe_directory(&engine_root, "sparrowzz/dss")?;
        let dataframes = read_safe_bounded(
            &dss_root,
            "docs/catalog/dataframes.json",
            limits.max_document_bytes,
        )?;
        let functions = read_safe_bounded(
            &dss_root,
            "docs/catalog/functions.json",
            limits.max_document_bytes,
        )?;
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
        for path in discover_documents(&dss_root, limits.max_documents)? {
            let relative = portable_relative(&dss_root, &path)?;
            let remaining = limits
                .max_total_document_bytes
                .checked_sub(total_document_bytes)
                .ok_or_else(|| limit_error("documentation total bytes"))?;
            let bytes = read_bounded_file(
                &path,
                limits.max_document_bytes.min(remaining),
                "documentation file or total size",
            )?;
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
        let bundles =
            discover_bundle_manifests(&dss_root, limits.max_examples, limits.max_example_bytes)?;
        let examples = examples::load_examples(&engine_root, &dss_root, &bundles, limits)?;
        let knowledge = EngineKnowledge::from_documents(&docs);
        let mut source_hashes = BTreeMap::new();
        insert_source_hash(
            &mut source_hashes,
            "sparrowzz/dss/docs/catalog/dataframes.json".to_string(),
            hash_bytes(&dataframes),
        )?;
        insert_source_hash(
            &mut source_hashes,
            "sparrowzz/dss/docs/catalog/functions.json".to_string(),
            hash_bytes(&functions),
        )?;
        for (path, bytes) in &documents {
            insert_source_hash(
                &mut source_hashes,
                format!("sparrowzz/dss/{path}"),
                hash_bytes(bytes),
            )?;
        }
        for example in &examples {
            for file in &example.files {
                insert_source_hash(
                    &mut source_hashes,
                    file.source_path.clone(),
                    file.sha256.clone(),
                )?;
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
            identity_part(
                &mut identity,
                b"example-category",
                example.category_path.join("/").as_bytes(),
            );
            identity_part(
                &mut identity,
                b"example-bundle",
                example.bundle_path.as_bytes(),
            );
            identity_part(&mut identity, b"example-tree", example.tree_hash.as_bytes());
            identity_part(
                &mut identity,
                b"example-importable",
                if example.importable { b"1" } else { b"0" },
            );
            for diagnostic in &example.diagnostics {
                identity_part(&mut identity, b"diagnostic", diagnostic.as_bytes());
            }
            if let Some(manifest) = &example.manifest {
                let manifest = serde_json::to_vec(manifest).map_err(|error| {
                    EngineResourceError::new("engine_resource_manifest_invalid", error.to_string())
                })?;
                identity_part(&mut identity, b"example-manifest", &manifest);
            }
            for file in &example.files {
                identity_part(
                    &mut identity,
                    b"example-file-path",
                    file.virtual_path.as_bytes(),
                );
                identity_part(&mut identity, b"example-file-role", file.role.as_bytes());
                identity_part(&mut identity, b"example-file-hash", file.sha256.as_bytes());
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
        mut snapshot: EngineResourceSnapshot,
        data_dir: &Path,
    ) -> Result<(EngineResourceSnapshot, PathBuf), EngineResourceError> {
        let parent = data_dir.join("system/engine-resources");
        std::fs::create_dir_all(&parent).map_err(EngineResourceError::io)?;
        let parent = parent.canonicalize().map_err(EngineResourceError::io)?;
        let destination = parent.join(&snapshot.id);
        let generated = generated_snapshot_files(&snapshot)?;
        let mut inventory = generated
            .iter()
            .map(|(path, bytes)| (path.clone(), hash_bytes(bytes)))
            .collect::<BTreeMap<_, _>>();
        inventory.extend(
            snapshot
                .source_hashes
                .iter()
                .map(|(path, hash)| (format!("source/{path}"), hash.clone())),
        );
        if destination.is_dir() {
            verify_published_snapshot(&destination, &snapshot.id, &inventory)?;
            rebase_snapshot(&mut snapshot, &destination)?;
            return Ok((snapshot, destination));
        }
        let staging = parent.join(format!(".{}-{}", snapshot.id, std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(EngineResourceError::io)?;
        }
        let result = (|| {
            std::fs::create_dir_all(&staging).map_err(EngineResourceError::io)?;
            for (relative, bytes) in &generated {
                write_published_file(&staging, relative, bytes)?;
            }
            for (relative, expected_hash) in &snapshot.source_hashes {
                let source = safe_file(&snapshot.engine_root, relative).map_err(|_| {
                    EngineResourceError::new(
                        "engine_resource_changed_during_scan",
                        format!("resource disappeared: {relative}"),
                    )
                })?;
                copy_verified_published_file(
                    &staging,
                    &format!("source/{relative}"),
                    &source,
                    expected_hash,
                )?;
            }
            write_published_file(
                &staging,
                "manifest.json",
                &serde_json::to_vec_pretty(&serde_json::json!({
                    "snapshotId": snapshot.id,
                    "engineContractVersion": snapshot.contract_version,
                    "catalogSchemaVersion": snapshot.catalog_schema_version,
                    "files": inventory,
                }))
                .map_err(|error| {
                    EngineResourceError::new("engine_resource_io", error.to_string())
                })?,
            )?;
            std::fs::rename(&staging, &destination).map_err(EngineResourceError::io)
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        verify_published_snapshot(&destination, &snapshot.id, &inventory)?;
        rebase_snapshot(&mut snapshot, &destination)?;
        Ok((snapshot, destination))
    }
}

fn generated_snapshot_files(
    snapshot: &EngineResourceSnapshot,
) -> Result<BTreeMap<String, Vec<u8>>, EngineResourceError> {
    let mut files = BTreeMap::new();
    for (name, bytes) in &snapshot.catalog_bytes {
        files.insert(format!("catalog/{name}"), bytes.clone());
    }
    for (relative, bytes) in &snapshot.documents {
        let relative = relative.strip_prefix("docs/").unwrap_or(relative);
        files.insert(format!("docs/{relative}"), bytes.clone());
    }
    files.insert(
        "docs-index.json".to_string(),
        serde_json::to_vec_pretty(&snapshot.knowledge.sections)
            .map_err(|error| EngineResourceError::new("engine_resource_io", error.to_string()))?,
    );
    files.insert(
        "examples.json".to_string(),
        serde_json::to_vec_pretty(&snapshot.examples)
            .map_err(|error| EngineResourceError::new("engine_resource_io", error.to_string()))?,
    );
    Ok(files)
}

fn write_published_file(
    root: &Path,
    relative: &str,
    bytes: &[u8],
) -> Result<(), EngineResourceError> {
    let path = published_path(root, relative)?;
    std::fs::write(path, bytes).map_err(EngineResourceError::io)
}

fn published_path(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            "invalid published resource path",
        ));
    }
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(EngineResourceError::io)?;
    }
    Ok(path)
}

fn copy_verified_published_file(
    root: &Path,
    relative: &str,
    source: &Path,
    expected_hash: &str,
) -> Result<(), EngineResourceError> {
    let destination = published_path(root, relative)?;
    let mut input = std::fs::File::open(source).map_err(|_| {
        EngineResourceError::new(
            "engine_resource_changed_during_scan",
            format!("resource unreadable: {relative}"),
        )
    })?;
    let mut output = std::fs::File::create(destination).map_err(EngineResourceError::io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|_| {
            EngineResourceError::new(
                "engine_resource_changed_during_scan",
                format!("resource unreadable: {relative}"),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut output, &buffer[..read]).map_err(EngineResourceError::io)?;
    }
    if format!("{:x}", hasher.finalize()) != expected_hash {
        return Err(EngineResourceError::new(
            "engine_resource_changed_during_scan",
            format!("resource changed: {relative}"),
        ));
    }
    Ok(())
}

fn verify_published_snapshot(
    destination: &Path,
    snapshot_id: &str,
    expected: &BTreeMap<String, String>,
) -> Result<(), EngineResourceError> {
    let manifest =
        std::fs::read(safe_file(destination, "manifest.json")?).map_err(EngineResourceError::io)?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).map_err(|error| {
        EngineResourceError::new("engine_resource_hash_mismatch", error.to_string())
    })?;
    if manifest
        .get("snapshotId")
        .and_then(serde_json::Value::as_str)
        != Some(snapshot_id)
    {
        return Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            "published snapshot id mismatch",
        ));
    }
    let declared: BTreeMap<String, String> =
        serde_json::from_value(manifest.get("files").cloned().ok_or_else(|| {
            EngineResourceError::new(
                "engine_resource_hash_mismatch",
                "published file inventory missing",
            )
        })?)
        .map_err(|error| {
            EngineResourceError::new("engine_resource_hash_mismatch", error.to_string())
        })?;
    if &declared != expected {
        return Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            "published file inventory mismatch",
        ));
    }
    for (relative, expected_hash) in expected {
        let path = safe_file(destination, relative).map_err(|_| {
            EngineResourceError::new(
                "engine_resource_hash_mismatch",
                format!("published file missing: {relative}"),
            )
        })?;
        if hash_file(&path)? != *expected_hash {
            return Err(EngineResourceError::new(
                "engine_resource_hash_mismatch",
                format!("published file changed: {relative}"),
            ));
        }
    }
    Ok(())
}

fn rebase_snapshot(
    snapshot: &mut EngineResourceSnapshot,
    destination: &Path,
) -> Result<(), EngineResourceError> {
    snapshot.engine_root = safe_directory(destination, "source")?;
    snapshot.source_root = safe_directory(&snapshot.engine_root, "sparrowzz/dss")?;
    Ok(())
}

pub(crate) fn hash_file(path: &Path) -> Result<String, EngineResourceError> {
    let mut file = std::fs::File::open(path).map_err(EngineResourceError::io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(EngineResourceError::io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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

fn read_safe_bounded(
    root: &Path,
    relative: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, EngineResourceError> {
    read_bounded_file(&safe_file(root, relative)?, max_bytes, "catalog file size")
}

pub(crate) fn read_bounded_file(
    path: &Path,
    max_bytes: u64,
    kind: &str,
) -> Result<Vec<u8>, EngineResourceError> {
    if std::fs::metadata(path)
        .map_err(EngineResourceError::io)?
        .len()
        > max_bytes
    {
        return Err(limit_error(kind));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(EngineResourceError::io)?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(EngineResourceError::io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(limit_error(kind));
    }
    Ok(bytes)
}

fn insert_source_hash(
    hashes: &mut BTreeMap<String, String>,
    path: String,
    hash: String,
) -> Result<(), EngineResourceError> {
    if let Some(existing) = hashes.get(&path) {
        if existing != &hash {
            return Err(EngineResourceError::new(
                "engine_resource_changed_during_scan",
                format!("resource changed while scanning: {path}"),
            ));
        }
        return Ok(());
    }
    hashes.insert(path, hash);
    Ok(())
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

fn discover_documents(
    dss_root: &Path,
    max_documents: usize,
) -> Result<Vec<PathBuf>, EngineResourceError> {
    let docs_root = safe_directory(dss_root, "docs")?;
    let mut files = Vec::new();
    walk_matching_files(&docs_root, true, max_documents, is_markdown, &mut files)?;
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    Ok(files)
}

fn discover_bundle_manifests(
    dss_root: &Path,
    max_bundles: usize,
    max_example_bytes: u64,
) -> Result<Vec<SourceFileSpec>, EngineResourceError> {
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
    walk_bundle_manifests(&root, max_bundles, &mut files)?;
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    files
        .into_iter()
        .map(|path| {
            Ok(SourceFileSpec {
                path: portable_relative(dss_root, &path)?,
                sha256: hash_bounded_file(&path, max_example_bytes, "example bytes")?,
            })
        })
        .collect()
}

fn walk_bundle_manifests(
    root: &Path,
    max_bundles: usize,
    manifests: &mut Vec<PathBuf>,
) -> Result<(), EngineResourceError> {
    let mut entries = std::fs::read_dir(root)
        .map_err(EngineResourceError::io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineResourceError::io)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(EngineResourceError::io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rssbundle"))
        {
            let manifest = path.join("manifest.json");
            if manifest.is_file() {
                if manifests.len() >= max_bundles {
                    return Err(limit_error("example count"));
                }
                manifests.push(manifest);
            }
            continue;
        }
        walk_bundle_manifests(&path, max_bundles, manifests)?;
    }
    Ok(())
}

fn walk_matching_files(
    root: &Path,
    reject_symlinks: bool,
    max_files: usize,
    matches: fn(&Path) -> bool,
    files: &mut Vec<PathBuf>,
) -> Result<(), EngineResourceError> {
    for entry in std::fs::read_dir(root).map_err(EngineResourceError::io)? {
        let entry = entry.map_err(EngineResourceError::io)?;
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
            walk_matching_files(&path, reject_symlinks, max_files, matches, files)?;
        } else if metadata.is_file() && matches(&path) {
            if files.len() >= max_files {
                return Err(limit_error("resource file count"));
            }
            files.push(path);
        }
    }
    Ok(())
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn hash_bounded_file(
    path: &Path,
    max_bytes: u64,
    kind: &str,
) -> Result<String, EngineResourceError> {
    read_bounded_file(path, max_bytes, kind).map(|bytes| hash_bytes(&bytes))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_inventory_hashes_fail_as_a_changed_scan() {
        let mut inventory = BTreeMap::new();
        insert_source_hash(&mut inventory, "shared/model.dss".into(), "first".into()).unwrap();
        let error = insert_source_hash(&mut inventory, "shared/model.dss".into(), "second".into())
            .unwrap_err();
        assert_eq!(error.code(), "engine_resource_changed_during_scan");
    }

    #[test]
    fn bounded_read_rejects_sparse_file_from_metadata() {
        use std::io::{Seek, SeekFrom, Write};

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.md");
        let mut file = std::fs::File::create(&path).unwrap();
        file.seek(SeekFrom::Start(1024 * 1024)).unwrap();
        file.write_all(&[0]).unwrap();
        let error = read_bounded_file(&path, 1024, "test file").unwrap_err();
        assert_eq!(error.code(), "engine_resource_limit_exceeded");
    }
}
