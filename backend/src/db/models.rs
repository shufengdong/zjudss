//! 表对应的 Rust 结构体：`Queryable`（读整行）与 `Insertable`（写，不含自增主键）。

// Diesel 2.x：派生宏（Queryable/Insertable）只从 crate 根重导出（默认 feature
// `with-deprecated`），prelude 仅含 trait 与 DSL；故显式导入。
use super::schema::{
    artifacts, auth_sessions, dataset_lineage_snapshots, dataset_views, draft_dataframes, messages,
    model_import_candidates, model_import_selections, model_imports, publish_receipts,
    result_analysis_inputs, result_analysis_runs, result_datasets, run_comparisons,
    script_resources, sessions, storage_scope_artifacts, storage_scopes, users,
    version_deletion_previews, workspace_drafts, workspaces,
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
    pub user_id: String,
    pub title_source: String,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct NewSession {
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub user_id: String,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = users)]
pub struct User {
    pub id: String,
    pub name: String,
    pub password_hash: String,
    pub storage_scope_id: i32,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub id: String,
    pub name: String,
    pub password_hash: String,
    pub storage_scope_id: i32,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = auth_sessions)]
pub struct AuthSession {
    pub token_hash: String,
    pub user_id: String,
    pub created_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub expires_at_ms: i64,
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
#[diesel(table_name = workspace_drafts)]
pub struct WorkspaceDraft {
    pub id: i32,
    pub workspace_id: i32,
    pub base_version_id: Option<i32>,
    pub revision: i64,
    pub diagnostics_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelImportStatus {
    Accepted,
    Processing,
    AwaitingResolution,
    ReadyToApply,
    Applied,
    Failed,
    Discarded,
}

impl ModelImportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Processing => "processing",
            Self::AwaitingResolution => "awaiting_resolution",
            Self::ReadyToApply => "ready_to_apply",
            Self::Applied => "applied",
            Self::Failed => "failed",
            Self::Discarded => "discarded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRef {
    pub id: i32,
    pub revision: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = workspace_drafts)]
pub struct NewWorkspaceDraft {
    pub workspace_id: i32,
    pub base_version_id: Option<i32>,
    pub revision: i64,
    pub diagnostics_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = draft_dataframes)]
pub struct DraftDataframe {
    pub id: i32,
    pub draft_id: i32,
    pub table_name: String,
    pub artifact_id: i32,
    pub schema_json: String,
    pub row_count: i64,
    pub content_hash: String,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = draft_dataframes)]
pub struct NewDraftDataframe {
    pub draft_id: i32,
    pub table_name: String,
    pub artifact_id: i32,
    pub schema_json: String,
    pub row_count: i64,
    pub content_hash: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = model_imports)]
pub struct ModelImport {
    pub id: i32,
    pub workspace_id: i32,
    pub session_id: i32,
    pub client_import_id: String,
    pub source_kind: String,
    pub source_summary_json: String,
    pub source_sha256: String,
    pub base_draft_id: Option<i32>,
    pub base_draft_revision: Option<i64>,
    pub status: String,
    pub stages_json: String,
    pub diagnostics_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = model_imports)]
pub struct NewModelImport {
    pub workspace_id: i32,
    pub session_id: i32,
    pub client_import_id: String,
    pub source_kind: String,
    pub source_summary_json: String,
    pub source_sha256: String,
    pub base_draft_id: Option<i32>,
    pub base_draft_revision: Option<i64>,
    pub status: String,
    pub stages_json: String,
    pub diagnostics_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = model_import_candidates)]
pub struct ModelImportCandidate {
    pub id: i32,
    pub import_id: i32,
    pub candidate_id: String,
    pub candidate_kind: String,
    pub display_name: String,
    pub table_name: Option<String>,
    pub artifact_id: Option<i32>,
    pub schema_json: Option<String>,
    pub row_count: Option<i64>,
    pub content_hash: Option<String>,
    pub evidence_json: String,
    pub selected: bool,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = model_import_candidates)]
pub struct NewModelImportCandidate {
    pub import_id: i32,
    pub candidate_id: String,
    pub candidate_kind: String,
    pub display_name: String,
    pub table_name: Option<String>,
    pub artifact_id: Option<i32>,
    pub schema_json: Option<String>,
    pub row_count: Option<i64>,
    pub content_hash: Option<String>,
    pub evidence_json: String,
    pub selected: bool,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = model_import_selections)]
pub struct ModelImportSelection {
    pub import_id: i32,
    pub candidate_id: String,
    pub table_name: Option<String>,
    pub selected: bool,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = publish_receipts)]
pub struct PublishReceipt {
    pub id: i32,
    pub draft_id: i32,
    pub draft_revision: i64,
    pub client_publish_id: String,
    pub version_id: i32,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = script_resources)]
pub struct ScriptResource {
    pub id: i32,
    pub session_id: i32,
    pub artifact_id: i32,
    pub name: String,
    pub source_kind: String,
    pub content_hash: String,
    pub validation_status: String,
    pub required_aliases_json: String,
    pub outputs_json: String,
    pub diagnostics_json: String,
    pub agent_bundle_version: Option<String>,
    pub agent_skill_refs_json: String,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = script_resources)]
pub struct NewScriptResource {
    pub session_id: i32,
    pub artifact_id: i32,
    pub name: String,
    pub source_kind: String,
    pub content_hash: String,
    pub validation_status: String,
    pub required_aliases_json: String,
    pub outputs_json: String,
    pub diagnostics_json: String,
    pub agent_bundle_version: Option<String>,
    pub agent_skill_refs_json: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = crate::db::schema::simulation_runs)]
pub struct SimulationRun {
    pub id: i32,
    pub session_id: i32,
    pub model_version_id: i32,
    pub model_content_hash: String,
    pub script_resource_id: i32,
    pub script_artifact_id: i32,
    pub script_content_hash: String,
    pub client_run_id: String,
    pub status: String,
    pub stage: String,
    pub engine_contract_version: String,
    pub agent_bundle_version: Option<String>,
    pub agent_skill_refs_json: String,
    pub exit_code: Option<i32>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub deleted_at_ms: Option<i64>,
    pub tombstone_json: Option<String>,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = dataset_lineage_snapshots)]
pub struct DatasetLineageSnapshot {
    pub id: i32,
    pub derived_dataset_id: i32,
    pub source_dataset_id: i32,
    pub source_name: String,
    pub source_schema_json: String,
    pub source_schema_hash: String,
    pub source_producer_json: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = version_deletion_previews)]
pub struct VersionDeletionPreviewRecord {
    pub token_hash: String,
    pub user_id: String,
    pub version_id: i32,
    pub impact_hash: String,
    pub impact_json: String,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone, serde::Serialize)]
#[diesel(table_name = result_datasets)]
#[serde(rename_all = "camelCase")]
pub struct ResultDataset {
    pub id: i32,
    pub run_id: Option<i32>,
    pub analysis_run_id: Option<i32>,
    pub artifact_id: Option<i32>,
    pub result_name: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
    pub tombstone_json: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = result_datasets)]
pub struct NewResultDataset {
    pub run_id: Option<i32>,
    pub analysis_run_id: Option<i32>,
    pub artifact_id: Option<i32>,
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
    pub source_dataset_id: i32,
    pub view_dataset_id: i32,
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
    pub source_name: String,
    pub source_schema_json: String,
    pub source_schema_hash: String,
    pub source_producer_json: String,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = dataset_views)]
pub struct NewDatasetView {
    pub source_dataset_id: i32,
    pub view_dataset_id: i32,
    pub name: String,
    pub kind: String,
    pub spec_json: String,
    pub is_default: bool,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone, serde::Serialize)]
#[diesel(table_name = run_comparisons)]
#[serde(rename_all = "camelCase")]
pub struct RunComparison {
    pub id: i32,
    pub session_id: i32,
    pub baseline_run_id: i32,
    pub candidate_run_id: i32,
    pub baseline_snapshot_json: String,
    pub candidate_snapshot_json: String,
    pub compatibility_json: String,
    pub analysis_run_id: i32,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = run_comparisons)]
pub struct NewRunComparison {
    pub session_id: i32,
    pub baseline_run_id: i32,
    pub candidate_run_id: i32,
    pub baseline_snapshot_json: String,
    pub candidate_snapshot_json: String,
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
    pub run_id: Option<i32>,
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
    pub run_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}
