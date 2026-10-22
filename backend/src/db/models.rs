//! 表对应的 Rust 结构体：`Queryable`（读整行）与 `Insertable`（写，不含自增主键）。

// Diesel 2.x：派生宏（Queryable/Insertable）只从 crate 根重导出（默认 feature
// `with-deprecated`），prelude 仅含 trait 与 DSL；故显式导入。
use super::schema::{
    artifacts, dataframe_bindings, dataset_views, ingestion_artifacts, ingestions, messages,
    model_versions, result_analysis_inputs, result_analysis_runs, result_datasets, sessions,
    simulation_tasks, storage_scope_artifacts, storage_scopes, task_comparisons, version_artifacts,
    workspaces,
};
#[allow(deprecated)]
use diesel::{Insertable, Queryable};

// ---- sessions ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct Session {
    pub id: i32,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub access_scope_kind: String,
    pub access_scope_key: String,
    pub storage_scope_id: Option<i32>,
    pub title_source: String,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct NewSession {
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub access_scope_kind: String,
    pub access_scope_key: String,
    pub storage_scope_id: Option<i32>,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = storage_scopes)]
pub struct StorageScope {
    pub id: i32,
    pub kind: String,
    pub scope_key: String,
    pub created_at_ms: i64,
    pub artifact_limit: i64,
    pub byte_limit: i64,
    pub warning_ratio: f64,
    pub used_artifacts: i64,
    pub used_bytes: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = storage_scopes)]
pub struct NewStorageScope {
    pub kind: String,
    pub scope_key: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = artifacts)]
pub struct Artifact {
    pub id: i32,
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub format: String,
    pub media_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = artifacts)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub format: String,
    pub media_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = storage_scope_artifacts)]
pub struct StorageScopeArtifact {
    pub storage_scope_id: i32,
    pub artifact_id: i32,
    pub ref_count: i64,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct ArtifactGcFailure {
    pub relative_path: String,
    pub error: String,
    pub attempts: i64,
    pub updated_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct Workspace {
    pub id: i32,
    pub session_id: i32,
    pub active_version_id: Option<i32>,
    pub draft_version_id: Option<i32>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = workspaces)]
pub struct NewWorkspace {
    pub session_id: i32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct ModelVersion {
    pub id: i32,
    pub workspace_id: i32,
    pub parent_version_id: Option<i32>,
    pub status: String,
    pub source_kind: String,
    pub origin_kind: String,
    pub manifest_json: String,
    pub patch_json: Option<String>,
    pub content_hash: String,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = model_versions)]
pub struct NewModelVersion {
    pub workspace_id: i32,
    pub parent_version_id: Option<i32>,
    pub status: String,
    pub source_kind: String,
    pub origin_kind: String,
    pub manifest_json: String,
    pub patch_json: Option<String>,
    pub content_hash: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct VersionArtifact {
    pub id: i32,
    pub version_id: i32,
    pub artifact_id: i32,
    pub role: String,
    pub relative_path: Option<String>,
    pub display_name: String,
    pub source_artifact_id: Option<i32>,
    pub sheet_name: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = version_artifacts)]
pub struct NewVersionArtifact {
    pub version_id: i32,
    pub artifact_id: i32,
    pub role: String,
    pub relative_path: Option<String>,
    pub display_name: String,
    pub source_artifact_id: Option<i32>,
    pub sheet_name: Option<String>,
}

#[derive(Queryable, Debug, Clone)]
pub struct DataframeBinding {
    pub id: i32,
    pub version_artifact_id: i32,
    pub table_kind: Option<String>,
    pub confidence: Option<f64>,
    pub resolution: String,
    pub columns_json: String,
    pub evidence_json: String,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = dataframe_bindings)]
pub struct NewDataframeBinding {
    pub version_artifact_id: i32,
    pub table_kind: Option<String>,
    pub confidence: Option<f64>,
    pub resolution: String,
    pub columns_json: String,
    pub evidence_json: String,
}

#[derive(Queryable, Debug, Clone)]
pub struct Ingestion {
    pub id: i32,
    pub workspace_id: i32,
    pub client_batch_id: String,
    pub origin_kind: String,
    pub source_ref_json: Option<String>,
    pub base_version_id: Option<i32>,
    pub published_version_id: Option<i32>,
    pub status: String,
    pub stages_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = ingestions)]
pub struct NewIngestion {
    pub workspace_id: i32,
    pub client_batch_id: String,
    pub origin_kind: String,
    pub source_ref_json: Option<String>,
    pub base_version_id: Option<i32>,
    pub published_version_id: Option<i32>,
    pub status: String,
    pub stages_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct IngestionArtifact {
    pub id: i32,
    pub ingestion_id: i32,
    pub client_file_id: String,
    pub artifact_id: i32,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub role: String,
    pub source_artifact_id: Option<i32>,
    pub sheet_name: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = ingestion_artifacts)]
pub struct NewIngestionArtifact {
    pub ingestion_id: i32,
    pub client_file_id: String,
    pub artifact_id: i32,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub role: String,
    pub source_artifact_id: Option<i32>,
    pub sheet_name: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

// ---- simulation_tasks ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = simulation_tasks)]
pub struct SimulationTask {
    pub id: i32,
    pub session_id: i32,
    pub status: String,
    pub stage: Option<String>,
    pub study_type: String,
    pub model_version_id: i32,
    pub study_spec_json: String,
    pub script_artifact_id: i32,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub engine_contract_version: String,
    pub exit_code: Option<i32>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = simulation_tasks)]
pub struct NewSimulationTask {
    pub session_id: i32,
    pub status: String,
    pub stage: Option<String>,
    pub study_type: String,
    pub model_version_id: i32,
    pub study_spec_json: String,
    pub script_artifact_id: i32,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub engine_contract_version: String,
    pub exit_code: Option<i32>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Queryable, Debug, Clone, serde::Serialize)]
#[diesel(table_name = result_datasets)]
#[serde(rename_all = "camelCase")]
pub struct ResultDataset {
    pub id: i32,
    pub task_id: Option<i32>,
    pub analysis_run_id: Option<i32>,
    pub artifact_id: i32,
    pub result_name: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = result_datasets)]
pub struct NewResultDataset {
    pub task_id: Option<i32>,
    pub analysis_run_id: Option<i32>,
    pub artifact_id: i32,
    pub result_name: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone, serde::Serialize)]
#[diesel(table_name = dataset_views)]
#[serde(rename_all = "camelCase")]
pub struct DatasetView {
    pub id: i32,
    pub dataset_id: i32,
    pub name: String,
    pub kind: String,
    pub spec_json: String,
    pub is_default: bool,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = result_analysis_runs)]
pub struct ResultAnalysisRun {
    pub id: i32,
    pub session_id: i32,
    pub request_message_id: Option<i32>,
    pub intent_json: String,
    pub script_artifact_id: i32,
    pub status: String,
    pub stage: String,
    pub error_json: Option<String>,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub engine_contract_version: String,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = result_analysis_runs)]
pub struct NewResultAnalysisRun {
    pub session_id: i32,
    pub request_message_id: Option<i32>,
    pub intent_json: String,
    pub script_artifact_id: i32,
    pub status: String,
    pub stage: String,
    pub error_json: Option<String>,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub engine_contract_version: String,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = result_analysis_inputs)]
pub struct ResultAnalysisInput {
    pub analysis_run_id: i32,
    pub dataset_id: i32,
    pub alias: String,
    pub ordinal: i32,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = dataset_views)]
pub struct NewDatasetView {
    pub dataset_id: i32,
    pub name: String,
    pub kind: String,
    pub spec_json: String,
    pub is_default: bool,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone, serde::Serialize)]
#[diesel(table_name = task_comparisons)]
#[serde(rename_all = "camelCase")]
pub struct TaskComparison {
    pub id: i32,
    pub session_id: i32,
    pub baseline_task_id: i32,
    pub candidate_task_id: i32,
    pub compatibility_json: String,
    pub analysis_run_id: i32,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = task_comparisons)]
pub struct NewTaskComparison {
    pub session_id: i32,
    pub baseline_task_id: i32,
    pub candidate_task_id: i32,
    pub compatibility_json: String,
    pub analysis_run_id: i32,
    pub created_at_ms: i64,
}

// ---- messages ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = messages)]
pub struct Message {
    pub id: i32,
    pub session_id: i32,
    pub task_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = messages)]
pub struct NewMessage {
    pub session_id: i32,
    pub task_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}
