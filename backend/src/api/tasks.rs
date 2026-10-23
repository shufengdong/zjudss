use super::ApiError;
use crate::application::{AppState, ms_to_iso};
use crate::auth::CurrentUser;
use crate::db::repositories::SimulationRepo;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
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
    p: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<TaskDto>, ApiError> {
    let t = SimulationRepo::new(s.pool).find(&p.id, id).await?;
    Ok(Json(TaskDto {
        id: t.id.to_string(),
        session_id: t.session_id.to_string(),
        status: t.status,
        stage: t.stage,
        study_type: t.study_type,
        study_spec: serde_json::from_str(&t.study_spec_json).unwrap_or(Value::Null),
        model_version_id: t.model_version_id.to_string(),
        script_artifact_id: t.script_artifact_id.to_string(),
        agent_bundle_version: t.agent_bundle_version,
        agent_skill_refs: serde_json::from_str(&t.agent_skill_refs_json)
            .unwrap_or_else(|_| serde_json::json!([])),
        engine_contract_version: t.engine_contract_version,
        error: t
            .error_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok()),
        created_at: ms_to_iso(t.created_at_ms),
        started_at: t.started_at_ms.map(ms_to_iso),
        finished_at: t.finished_at_ms.map(ms_to_iso),
    }))
}

#[derive(Deserialize)]
pub struct TaskListQuery {
    limit: Option<i64>,
}

pub async fn list_session(
    State(s): State<AppState>,
    p: CurrentUser,
    Path(session_id): Path<i32>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<Vec<TaskDto>>, ApiError> {
    let tasks = s
        .history_service
        .list_simulations(&p, session_id, query.limit.unwrap_or(50))
        .await?;
    Ok(Json(tasks.into_iter().map(task_dto).collect()))
}

fn task_dto(t: crate::db::models::SimulationTask) -> TaskDto {
    TaskDto {
        id: t.id.to_string(),
        session_id: t.session_id.to_string(),
        status: t.status,
        stage: t.stage,
        study_type: t.study_type,
        study_spec: serde_json::from_str(&t.study_spec_json).unwrap_or(Value::Null),
        model_version_id: t.model_version_id.to_string(),
        script_artifact_id: t.script_artifact_id.to_string(),
        agent_bundle_version: t.agent_bundle_version,
        agent_skill_refs: serde_json::from_str(&t.agent_skill_refs_json)
            .unwrap_or_else(|_| serde_json::json!([])),
        engine_contract_version: t.engine_contract_version,
        error: t
            .error_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok()),
        created_at: ms_to_iso(t.created_at_ms),
        started_at: t.started_at_ms.map(ms_to_iso),
        finished_at: t.finished_at_ms.map(ms_to_iso),
    }
}
pub async fn datasets(
    State(s): State<AppState>,
    p: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Vec<crate::application::datasets::DatasetMetadata>>, ApiError> {
    Ok(Json(s.dataset_service.list(&p, id).await?))
}
