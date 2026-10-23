use serde::Serialize;

use crate::agent::knowledge::{AgentKnowledge, KnowledgeError};
use crate::api::{RouteContract, route_manifest};
use crate::application::{MESSAGE_TYPES, visualization::VISUALIZATION_KINDS};
use crate::tools::registered_tool_names;

pub const UPLOAD_FORMATS: &[&str] = &[
    "dss",
    "dssbundle",
    "rssbundle",
    "zip",
    "csv",
    "excel",
    "parquet",
    "rss_script",
];

pub const STABLE_ERROR_CODES: &[&str] = &[
    "invalid_credentials",
    "authentication_required",
    "internal_error",
    "not_found",
    "agent_queue_full",
    "storage_quota_exceeded",
    "upload_too_large",
    "idempotency_conflict",
    "draft_revision_conflict",
    "model_import_unsupported",
    "model_import_resolution_required",
    "invalid_script",
    "script_validation_failed",
    "active_model_changed",
    "script_static_invalid",
    "script_model_incompatible",
    "run_queue_full",
    "invalid_run",
    "engine_resource_snapshot_mismatch",
    "case_not_modelizable",
    "invalid_model_version",
    "draft_discard_confirmation_required",
    "active_version_cannot_be_deleted",
    "version_has_active_consumers",
    "deletion_impact_changed",
    "invalid_deletion_impact_token",
    "invalid_dataset_page",
    "result_busy",
    "history_error",
    "history_page_invalid",
    "history_query_invalid",
    "history_search_unavailable",
    "analysis_alias_invalid",
    "analysis_create_failed",
    "analysis_engine_failed",
    "analysis_event_failed",
    "analysis_input_not_found",
    "analysis_inputs_invalid",
    "analysis_not_found",
    "analysis_output_commit_failed",
    "analysis_output_invalid",
    "analysis_output_mismatch",
    "analysis_outputs_required",
    "analysis_publish_failed",
    "analysis_script_commit_failed",
    "analysis_script_missing",
    "analysis_script_unsafe",
    "analysis_staging_failed",
    "analysis_update_failed",
    "dataset_not_found",
    "dataset_schema_invalid",
    "dataset_view_read_failed",
    "storage_unavailable",
    "view_create_failed",
    "view_materialization_failed",
    "visualization_column_missing",
    "visualization_dtype_invalid",
    "visualization_kind_invalid",
    "visualization_spec_invalid",
    "visualization_too_large",
    "visualization_unsafe",
    "comparison_event_failed",
    "invalid_model_patch",
    "comparison_incompatible",
    "comparison_publish_failed",
    "artifact_error",
    "old_value_mismatch",
    "row_not_unique",
    "table_not_found",
    "unit_incompatible",
    "workspace_error",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractManifest {
    pub routes: Vec<RouteContract>,
    pub message_types: Vec<&'static str>,
    pub tool_names: Vec<&'static str>,
    pub skill_ids: Vec<String>,
    pub upload_sources: Vec<&'static str>,
    pub visualization_kinds: Vec<&'static str>,
    pub stable_error_codes: Vec<&'static str>,
}

pub fn contract_manifest() -> Result<ContractManifest, KnowledgeError> {
    let knowledge = AgentKnowledge::load_embedded("v3.0.0")?;
    let mut skill_ids = knowledge
        .skill_index()
        .map(|skill| skill.id.clone())
        .collect::<Vec<_>>();
    skill_ids.sort();

    Ok(ContractManifest {
        routes: route_manifest(),
        message_types: MESSAGE_TYPES.to_vec(),
        tool_names: registered_tool_names().to_vec(),
        skill_ids,
        upload_sources: UPLOAD_FORMATS.to_vec(),
        visualization_kinds: VISUALIZATION_KINDS.to_vec(),
        stable_error_codes: STABLE_ERROR_CODES.to_vec(),
    })
}
