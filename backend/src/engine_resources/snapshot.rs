use std::collections::BTreeMap;
use std::path::PathBuf;

use super::examples::EngineExample;
use super::facades::FacadeRegistry;
use super::knowledge::EngineKnowledge;
use crate::catalog::SchemaRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSchemaVersions {
    pub functions: u32,
    pub study_facades: u32,
    pub dataframes: u32,
    pub dtype_grammar: u32,
}

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
    pub catalog_schema_versions: CatalogSchemaVersions,
    pub dataframes_sha256: String,
    pub functions_sha256: String,
    pub study_facades_sha256: String,
    pub source_root: PathBuf,
    pub engine_root: PathBuf,
    pub catalog_json: serde_json::Value,
    pub functions_json: serde_json::Value,
    pub study_facades_json: serde_json::Value,
    pub schemas: SchemaRegistry,
    pub facades: FacadeRegistry,
    pub catalog_bytes: BTreeMap<String, Vec<u8>>,
    pub documents: BTreeMap<String, Vec<u8>>,
    pub knowledge: EngineKnowledge,
    pub examples: Vec<EngineExample>,
    pub(crate) source_hashes: BTreeMap<String, String>,
}

impl EngineResourceSnapshot {
    pub(crate) fn minimal(
        id: impl Into<String>,
        contract_version: impl Into<String>,
        source_root: PathBuf,
    ) -> Self {
        let catalog_json = serde_json::json!({
            "schema_version": 3,
            "frames": {
                "table": {"contract": {
                    "kind":"model", "rowCount":"zero_or_more", "shape":"uniform",
                    "columnPolicy":{"mode":"open","unknownColumns":"ignored"},
                    "columns":[{"name":"id","dtype":"str","required":true,"nullable":false,"unit":"not_applicable"}],
                    "stableKeys":["id"], "sortKeys":["id"], "columnAliases":{},
                    "allowedConversions":[], "rules":[], "source":["zjudss://test"]
                }}
            },
            "result_frames": {
                "result": {"contract": {
                    "kind":"result", "rowCount":"zero_or_more", "shape":"uniform",
                    "columnPolicy":{"mode":"closed","unknownColumns":"forbidden"},
                    "columns":[{"name":"id","dtype":"str","required":true,"nullable":false,"unit":"not_applicable"}],
                    "stableKeys":[], "sortKeys":[], "columnAliases":{},
                    "allowedConversions":[], "rules":[], "source":["zjudss://test"]
                }}
            },
            "dtype_grammar": {
                "version":1,
                "primitives":{"str":{"arrow":"Utf8"}},
                "constructors":{}, "semantic_types":{}, "upload_conversions":{}
            }
        });
        let study_facades_json = serde_json::json!({
            "schema_version":1,
            "facades":[{
                "id":"test", "function":"test", "family":"test", "title":"测试",
                "description":"测试门面", "differences":["测试用途"],
                "inputs":[{"id":"table","alias":"table","inputKind":"model","schemaRef":"frames.table","required":true,"position":1}],
                "results":[{"id":"result","name":"result","schemaRef":"result_frames.result","description":"测试结果","recommended":true}],
                "resultConstraints":[{"type":"fixed_results","results":["result"],"bindingCount":1}]
            }]
        });
        let functions_json = serde_json::json!({"schema_version":2,"frame_functions":[]});
        let mut schemas = SchemaRegistry::from_catalog(&catalog_json)
            .expect("embedded minimal schema catalog must be valid");
        let facades = FacadeRegistry::from_catalogs(&study_facades_json, &mut schemas)
            .expect("embedded minimal facade catalog must be valid");
        Self {
            id: id.into(),
            contract_version: contract_version.into(),
            catalog_schema_versions: CatalogSchemaVersions {
                functions: 2,
                study_facades: 1,
                dataframes: 3,
                dtype_grammar: 1,
            },
            dataframes_sha256: String::new(),
            functions_sha256: String::new(),
            study_facades_sha256: String::new(),
            source_root: source_root.clone(),
            engine_root: source_root,
            catalog_json,
            functions_json,
            study_facades_json,
            schemas,
            facades,
            catalog_bytes: BTreeMap::new(),
            documents: BTreeMap::new(),
            knowledge: EngineKnowledge::from_documents(&[]),
            examples: Vec::new(),
            source_hashes: BTreeMap::new(),
        }
    }
}
