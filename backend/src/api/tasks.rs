use super::ApiError;
use crate::application::{AppState, ms_to_iso};
use crate::auth::CurrentPrincipal;
use crate::db::repositories::SimulationRepo;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    id: String,
    session_id: String,
    status: String,
    stage: Option<String>,
    study_type: String,
    study_spec: Value,
    model_version_id: String,
    script_artifact_id: String,
    agent_bundle_version: String,
    agent_skill_refs: Value,
    engine_contract_version: String,
    error: Option<Value>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}
pub async fn get(
    State(s): State<AppState>,
    CurrentPrincipal(p): CurrentPrincipal,
    Path(id): Path<i32>,
) -> Result<Json<TaskDto>, ApiError> {
    let t = SimulationRepo::new(s.pool).find(&p.access, id).await?;
    Ok(Json(TaskDto {
        id: t.id.to_string(),
        session_id: t.session_id.to_string(),
        status: t.status,
        stage: t.stage,
        study_type: t.study_type.unwrap_or_default(),
        study_spec: t
            .study_spec_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or(Value::Null),
        model_version_id: t
            .model_version_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        script_artifact_id: t
            .script_artifact_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        agent_bundle_version: t.agent_bundle_version.unwrap_or_default(),
        agent_skill_refs: t
            .agent_skill_refs_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_else(|| serde_json::json!([])),
        engine_contract_version: t.engine_contract_version.unwrap_or_default(),
        error: t
            .error_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok()),
        created_at: ms_to_iso(t.created_at_ms),
        started_at: t.started_at_ms.map(ms_to_iso),
        finished_at: t.finished_at_ms.map(ms_to_iso),
    }))
}
pub async fn datasets(
    State(s): State<AppState>,
    CurrentPrincipal(p): CurrentPrincipal,
    Path(id): Path<i32>,
) -> Result<Json<Vec<crate::application::datasets::DatasetMetadata>>, ApiError> {
    Ok(Json(s.dataset_service.list(&p, id).await?))
}
