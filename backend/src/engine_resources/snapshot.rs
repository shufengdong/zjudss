use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::examples::EngineExample;
use super::knowledge::EngineKnowledge;

#[derive(Debug, Clone)]
pub struct ResourceContract {
    pub contract_version: String,
    pub catalog_schema_version: u32,
    pub dataframes_sha256: String,
    pub functions_sha256: String,
    pub docs_tree_sha256: String,
    pub doc_paths: Vec<String>,
    pub root_rss: Vec<SourceFileSpec>,
    pub bundles: Vec<SourceFileSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFileSpec {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct EngineResourceSnapshot {
    pub id: String,
    pub contract_version: String,
    pub catalog_schema_version: u32,
    pub source_root: PathBuf,
    pub engine_root: PathBuf,
    pub catalog_json: serde_json::Value,
    pub functions_json: serde_json::Value,
    pub catalog_bytes: BTreeMap<String, Vec<u8>>,
    pub documents: BTreeMap<String, Vec<u8>>,
    pub knowledge: EngineKnowledge,
    pub examples: Vec<EngineExample>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmbeddedExampleSources {
    pub schema_version: u32,
    pub source_commit: String,
    pub root_rss: Vec<SourceFileSpec>,
    pub bundles: Vec<SourceFileSpec>,
}
