use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{ActivateVersionRequest, AppState, PublishDraftRequest, ms_to_iso};
use crate::auth::CurrentUser;
use crate::db::repositories::{DraftSnapshot, VersionSnapshot, WorkspaceSnapshot};

use super::ApiError;

pub async fn workspace(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(workspace_json(
        state.version_service.workspace(&user, session_id).await?,
    )))
}

pub async fn versions(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let versions = state.version_service.versions(&user, session_id).await?;
    Ok(Json(
        json!({"versions": versions.into_iter().map(version_json).collect::<Vec<_>>() }),
    ))
}

pub async fn publish(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Json(request): Json<PublishDraftRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let published = state
        .version_service
        .publish(&user, session_id, request)
        .await?;
    let status = if published.replayed || published.deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(
            json!({"version":version_json(published.version),"deduplicated":published.deduplicated,"replayed":published.replayed}),
        ),
    ))
}

pub async fn version(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(version_json(
        state.version_service.version(&user, version_id).await?,
    )))
}

pub async fn activate(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
    Json(request): Json<ActivateVersionRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(workspace_json(
        state
            .version_service
            .activate(&user, version_id, request)
            .await?,
    )))
}

pub async fn deletion_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(
            state
                .version_service
                .preview_deletion(&user, version_id)
                .await?,
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    impact_token: String,
}

pub async fn delete_version(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
    Query(query): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    state
        .version_service
        .delete(&user, version_id, query.impact_token)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn workspace_json(snapshot: WorkspaceSnapshot) -> Value {
    json!({
        "id": snapshot.workspace.id.to_string(),
        "sessionId": snapshot.workspace.session_id.to_string(),
        "active": snapshot.active.map(version_json),
        "draft": snapshot.draft.map(draft_json),
        "activeModelImports": []
    })
}

fn draft_json(snapshot: DraftSnapshot) -> Value {
    json!({
        "id": snapshot.draft.id.to_string(),
        "workspaceId": snapshot.draft.workspace_id.to_string(),
        "baseVersionId": snapshot.draft.base_version_id.map(|id| id.to_string()),
        "revision": snapshot.draft.revision,
        "diagnostics": serde_json::from_str::<Value>(&snapshot.draft.diagnostics_json).unwrap_or_else(|_| json!({})),
        "tables": snapshot.dataframes.into_iter().map(|table| json!({
            "id":table.id.to_string(),"tableName":table.table_name,"artifactId":table.artifact_id.to_string(),
            "schema":serde_json::from_str::<Value>(&table.schema_json).unwrap_or_else(|_| json!([])),
            "rowCount":table.row_count,"contentHash":table.content_hash
        })).collect::<Vec<_>>(),
        "createdAt":ms_to_iso(snapshot.draft.created_at_ms),"updatedAt":ms_to_iso(snapshot.draft.updated_at_ms)
    })
}

fn version_json(snapshot: VersionSnapshot) -> Value {
    json!({
        "id":snapshot.version.id.to_string(),"workspaceId":snapshot.version.workspace_id.to_string(),
        "contentHash":snapshot.version.content_hash,
        "sourceSummary":serde_json::from_str::<Value>(&snapshot.version.source_summary_json).unwrap_or_else(|_| json!({})),
        "engineContractVersion":snapshot.version.engine_contract_version,
        "tables":snapshot.dataframes.into_iter().map(|table| json!({
            "id":table.id.to_string(),"tableName":table.table_name,"artifactId":table.artifact_id.to_string(),
            "schema":serde_json::from_str::<Value>(&table.schema_json).unwrap_or_else(|_| json!([])),
            "rowCount":table.row_count,"contentHash":table.content_hash
        })).collect::<Vec<_>>(),
        "createdAt":ms_to_iso(snapshot.version.created_at_ms)
    })
}
