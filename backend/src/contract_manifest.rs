use serde::Serialize;

use crate::agent::knowledge::{AgentKnowledge, KnowledgeError};
use crate::api::{RouteContract, route_manifest};
use crate::application::{MESSAGE_TYPES, visualization::VISUALIZATION_KINDS};
use crate::ingestion::UPLOAD_SOURCES;
use crate::tools::registered_tool_names;

pub const STABLE_ERROR_CODES: &[&str] = &[
    "archive_invalid",
    "archive_unsafe_path",
    "artifact_too_large",
    "unsupported_file_type",
    "idempotency_conflict",
    "dataframe_schema_unknown",
    "dataframe_schema_ambiguous",
    "duplicate_table_requires_confirmation",
    "missing_dependency",
    "missing_required_table",
    "model_input_not_found",
    "version_conflict",
    "invalid_model_patch",
    "unsupported_study",
    "model_validation_failed",
    "solver_failed",
    "not_converged",
    "comparison_incompatible",
    "result_dataset_not_found",
    "result_schema_mismatch",
    "analysis_script_rejected",
    "analysis_output_limit_exceeded",
    "dataset_view_not_found",
    "invalid_visualization_spec",
    "visualization_data_too_large",
    "agent_queue_full",
    "invalid_cursor",
    "invalid_pagination",
    "history_query_too_long",
    "history_search_unavailable",
    "storage_quota_exceeded",
    "engine_resource_unavailable",
    "engine_contract_mismatch",
    "example_not_found",
    "example_not_importable",
    "example_source_changed",
    "engine_feature_unavailable",
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
    let knowledge = AgentKnowledge::load_embedded("v2.0.0")?;
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
        upload_sources: UPLOAD_SOURCES.to_vec(),
        visualization_kinds: VISUALIZATION_KINDS.to_vec(),
        stable_error_codes: STABLE_ERROR_CODES.to_vec(),
    })
}
