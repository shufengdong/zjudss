use super::ApiError;
use crate::application::{AppState, CreateRunRequest, ms_to_iso};
use crate::auth::CurrentUser;
use crate::db::models::SimulationRun;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(script_id): Path<i32>,
    Json(request): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let submitted = state.run_service.create(&user, script_id, request).await?;
    let status = if submitted.replayed {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((
        status,
        Json(json!({"run":run_json(submitted.run),"replayed":submitted.replayed})),
    ))
}
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(run_json(state.run_service.find(&user, id).await?)))
}
#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
}
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let runs = state
        .run_service
        .list(&user, session_id, query.limit.unwrap_or(50))
        .await?;
    Ok(Json(
        json!({"runs":runs.into_iter().map(run_json).collect::<Vec<_>>() }),
    ))
}
pub async fn datasets(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    state.run_service.find(&user, id).await?;
    Ok(Json(
        json!({"datasets":state.dataset_service.list(&user,id).await?}),
    ))
}
fn run_json(run: SimulationRun) -> Value {
    json!({"id":run.id.to_string(),"sessionId":run.session_id.to_string(),"modelVersionId":run.model_version_id.to_string(),"modelContentHash":run.model_content_hash,"scriptResourceId":run.script_resource_id.to_string(),"scriptArtifactId":run.script_artifact_id.to_string(),"scriptContentHash":run.script_content_hash,"clientRunId":run.client_run_id,"status":run.status,"stage":run.stage,"engineContractVersion":run.engine_contract_version,"agentBundleVersion":run.agent_bundle_version,"agentSkillRefs":serde_json::from_str::<Value>(&run.agent_skill_refs_json).unwrap_or_else(|_|json!([])),"exitCode":run.exit_code,"error":run.error_json.as_deref().and_then(|value|serde_json::from_str::<Value>(value).ok()),"createdAt":ms_to_iso(run.created_at_ms),"startedAt":run.started_at_ms.map(ms_to_iso),"finishedAt":run.finished_at_ms.map(ms_to_iso)})
}
