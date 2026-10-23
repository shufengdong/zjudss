mod examples;
mod knowledge;
mod snapshot;

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::agent::knowledge::AgentKnowledge;

pub use examples::{EngineExample, ExampleSourceFile};
pub use knowledge::{DocSearchHit, DocSection, EngineKnowledge};
use snapshot::EmbeddedExampleSources;
pub use snapshot::{EngineResourceSnapshot, ResourceContract, SourceFileSpec};

const EXAMPLE_SOURCES: &str = include_str!("../../resources/agent/v2/examples-source.json");
const DOC_PATHS: [&str; 11] = [
    "docs/README.md",
    "docs/agent_quickstart.md",
    "docs/dataframe_schema_reference.md",
    "docs/dsa_function_reference.md",
    "docs/dss_to_rss.md",
    "docs/opendss_compatibility_matrix.md",
    "docs/opendss_to_rss.md",
    "docs/rsdss_function_reference.md",
    "docs/rss_language_reference.md",
    "docs/simulation_recipes.md",
    "docs/validation_and_troubleshooting.md",
];

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
    pub fn embedded_contract() -> Result<ResourceContract, EngineResourceError> {
        let knowledge = AgentKnowledge::load_embedded("v2.0.1").map_err(|error| {
            EngineResourceError::new("agent_knowledge_invalid", error.to_string())
        })?;
        let examples: EmbeddedExampleSources =
            serde_json::from_str(EXAMPLE_SOURCES).map_err(|error| {
                EngineResourceError::new("examples_contract_invalid", error.to_string())
            })?;
        if examples.schema_version != 1
            || examples.source_commit != knowledge.manifest.sparrowzz_commit
        {
            return Err(EngineResourceError::new(
                "engine_contract_mismatch",
                "案例来源版本不匹配",
            ));
        }
        verify_bytes_hash(
            EXAMPLE_SOURCES.as_bytes(),
            &knowledge.manifest.examples_source_sha256,
        )?;
        Ok(ResourceContract {
            contract_version: knowledge.manifest.engine_contract_version,
            catalog_schema_version: knowledge.manifest.catalog_schema_version,
            dataframes_sha256: knowledge
                .manifest
                .catalogs
                .get("dataframes")
                .cloned()
                .ok_or_else(|| {
                    EngineResourceError::new("engine_contract_invalid", "缺少 dataframes hash")
                })?,
            functions_sha256: knowledge
                .manifest
                .catalogs
                .get("functions")
                .cloned()
                .ok_or_else(|| {
                    EngineResourceError::new("engine_contract_invalid", "缺少 functions hash")
                })?,
            docs_tree_sha256: knowledge.manifest.docs_tree_sha256,
            doc_paths: DOC_PATHS.iter().map(|path| path.to_string()).collect(),
            root_rss: examples.root_rss,
            bundles: examples.bundles,
        })
    }

    pub fn load(tensoreval_cwd: &Path) -> Result<EngineResourceSnapshot, EngineResourceError> {
        Self::load_with_contract(tensoreval_cwd, Self::embedded_contract()?, &HashSet::new())
    }

    pub fn load_with_contract(
        tensoreval_cwd: &Path,
        contract: ResourceContract,
        capabilities: &HashSet<String>,
    ) -> Result<EngineResourceSnapshot, EngineResourceError> {
        let engine_root = tensoreval_cwd
            .canonicalize()
            .map_err(EngineResourceError::io)?;
        let dss_root = safe_directory(&engine_root, "sparrowzz/dss")?;
        let dataframes_path = safe_file(&dss_root, "docs/catalog/dataframes.json")?;
        let functions_path = safe_file(&dss_root, "docs/catalog/functions.json")?;
        let dataframes = std::fs::read(&dataframes_path).map_err(EngineResourceError::io)?;
        let functions = std::fs::read(&functions_path).map_err(EngineResourceError::io)?;
        verify_bytes_hash(&dataframes, &contract.dataframes_sha256)?;
        verify_bytes_hash(&functions, &contract.functions_sha256)?;
        let catalog_json: serde_json::Value = serde_json::from_slice(&dataframes)
            .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
        let functions_json: serde_json::Value = serde_json::from_slice(&functions)
            .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
        if catalog_json
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(contract.catalog_schema_version as u64)
            || functions_json
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                != Some(contract.catalog_schema_version as u64)
        {
            return Err(EngineResourceError::new(
                "engine_contract_mismatch",
                "catalog schema version 不匹配",
            ));
        }

        let mut docs = Vec::new();
        let mut documents = BTreeMap::new();
        let mut docs_hasher = Sha256::new();
        for relative in &contract.doc_paths {
            let path = safe_file(&dss_root, relative)?;
            let bytes = std::fs::read(&path).map_err(EngineResourceError::io)?;
            docs_hasher.update(relative.as_bytes());
            docs_hasher.update(b"\0");
            docs_hasher.update(&bytes);
            docs_hasher.update(b"\0");
            let content = String::from_utf8(bytes).map_err(|error| {
                EngineResourceError::new("engine_doc_invalid_utf8", error.to_string())
            })?;
            documents.insert(relative.clone(), content.as_bytes().to_vec());
            docs.push((relative.clone(), content));
        }
        let docs_hash = format!("{:x}", docs_hasher.finalize());
        if docs_hash != contract.docs_tree_sha256 {
            return Err(EngineResourceError::new(
                "engine_resource_hash_mismatch",
                "docs tree hash 不匹配",
            ));
        }
        let examples = examples::load_examples(
            &engine_root,
            &dss_root,
            &contract.root_rss,
            &contract.bundles,
            capabilities,
        )?;
        let knowledge = EngineKnowledge::from_documents(&docs);
        let mut identity = Sha256::new();
        identity.update(contract.contract_version.as_bytes());
        identity.update(contract.dataframes_sha256.as_bytes());
        identity.update(contract.functions_sha256.as_bytes());
        identity.update(contract.docs_tree_sha256.as_bytes());
        for example in &examples {
            identity.update(example.id.as_bytes());
            identity.update(example.tree_hash.as_bytes());
        }
        let id = format!("ers_{}", &format!("{:x}", identity.finalize())[..32]);
        Ok(EngineResourceSnapshot {
            id,
            contract_version: contract.contract_version,
            catalog_schema_version: contract.catalog_schema_version,
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
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn verify_bytes_hash(bytes: &[u8], expected: &str) -> Result<(), EngineResourceError> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            format!("期望 {expected}，实际 {actual}"),
        ))
    }
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
