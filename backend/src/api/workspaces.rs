use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::application::{AppState, WorkspaceError, ms_to_iso};
use crate::auth::{AccessScope, CurrentPrincipal};
use crate::db::models::ModelVersion;

use super::ApiError;

pub async fn workspace(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let record = state
        .workspace_service
        .get_or_create(&principal.access, session_id)
        .await?;
    let active = version_json(&state, &principal.access, record.active).await?;
    let draft = version_json(&state, &principal.access, record.draft).await?;
    let active_ingestions = state
        .ingestion_service
        .active(&principal, record.workspace.id)
        .await?;
    Ok(Json(json!({
        "id": record.workspace.id.to_string(),
        "sessionId": record.workspace.session_id.to_string(),
        "active": active,
        "draft": draft,
        "activeIngestions": active_ingestions,
    })))
}

pub async fn versions(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    state
        .workspace_service
        .get_or_create(&principal.access, session_id)
        .await?;
    let versions = state
        .workspace_service
        .list_versions(&principal.access, session_id)
        .await?;
    let mut items = Vec::with_capacity(versions.len());
    for version in versions {
        items.push(
            version_json(&state, &principal.access, Some(version))
                .await?
                .unwrap(),
        );
    }
    Ok(Json(json!({ "versions": items })))
}

pub async fn version(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(version_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let version = state
        .workspace_service
        .find_version(&principal.access, version_id)
        .await?;
    Ok(Json(
        version_json(&state, &principal.access, Some(version))
            .await?
            .unwrap(),
    ))
}

async fn version_json(
    state: &AppState,
    scope: &AccessScope,
    version: Option<ModelVersion>,
) -> Result<Option<Value>, WorkspaceError> {
    let Some(version) = version else {
        return Ok(None);
    };
    let manifest: Value =
        serde_json::from_str(&version.manifest_json).unwrap_or_else(|_| json!({}));
    let mut inputs = Vec::new();
    for (input, binding) in state.workspace_service.inputs(scope, version.id).await? {
        let evidence = binding
            .as_ref()
            .and_then(|b| serde_json::from_str::<Value>(&b.evidence_json).ok())
            .unwrap_or_else(|| json!({}));
        inputs.push(json!({
            "inputId": input.id.to_string(),
            "artifactId": input.artifact_id.to_string(),
            "fileName": input.display_name,
            "sheetName": input.sheet_name,
            "tableKind": binding.as_ref().and_then(|b| b.table_kind.clone()),
            "confidence": binding.as_ref().and_then(|b| b.confidence),
            "resolution": binding.as_ref().map(|b| b.resolution.as_str()).unwrap_or("pending"),
            "candidates": evidence.get("candidates").cloned().unwrap_or_else(|| json!([])),
            "matchedRequiredColumns": evidence.get("matchedRequiredColumns").cloned().unwrap_or_else(|| json!([])),
            "missingRequiredColumns": evidence.get("missingRequiredColumns").cloned().unwrap_or_else(|| json!([])),
            "typeConflicts": evidence.get("typeConflicts").cloned().unwrap_or_else(|| json!([])),
        }));
    }
    Ok(Some(json!({
        "versionId": version.id.to_string(),
        "parentVersionId": version.parent_version_id.map(|id| id.to_string()),
        "sourceKind": version.source_kind,
        "originKind": version.origin_kind,
        "origin": manifest.get("origin").cloned(),
        "status": version.status,
        "inputs": inputs,
        "missing": manifest.get("missing").cloned().unwrap_or_else(|| json!([])),
        "diagnostics": manifest.get("diagnostics").cloned().unwrap_or_else(|| json!([])),
        "createdAt": ms_to_iso(version.created_at_ms),
    })))
}
