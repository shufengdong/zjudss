use std::collections::BTreeMap;
use std::path::PathBuf;

use super::examples::EngineExample;
use super::knowledge::EngineKnowledge;

#[derive(Debug, Clone)]
pub(crate) struct SourceFileSpec {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    pub max_documents: usize,
    pub max_document_bytes: u64,
    pub max_total_document_bytes: u64,
    pub max_examples: usize,
    pub max_example_files: usize,
    pub max_example_bytes: u64,
    pub max_dependency_depth: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_documents: 512,
            max_document_bytes: 4 * 1024 * 1024,
            max_total_document_bytes: 64 * 1024 * 1024,
            max_examples: 2_048,
            max_example_files: 10_000,
            max_example_bytes: 2 * 1024 * 1024 * 1024,
            max_dependency_depth: 64,
        }
    }
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
    pub(crate) source_hashes: BTreeMap<String, String>,
}
