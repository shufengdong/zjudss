use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::application::AppState;
use crate::auth::CurrentPrincipal;

use super::ApiError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageResponse {
    scope: String,
    artifact_limit: i64,
    byte_limit: i64,
    warning_ratio: f64,
    artifact_count: i64,
    logical_bytes: i64,
    is_warning: bool,
    is_exceeded: bool,
}

pub async fn usage(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
) -> Result<Json<StorageUsageResponse>, ApiError> {
    let scope = state
        .artifact_service
        .resolve_scope(&principal.storage)
        .await?;
    let usage = state.artifact_service.get_usage(scope.id).await?;
    let artifact_ratio = usage.used_artifacts as f64 / usage.artifact_limit as f64;
    let byte_ratio = usage.used_bytes as f64 / usage.byte_limit as f64;
    Ok(Json(StorageUsageResponse {
        scope: principal.storage.kind,
        artifact_limit: usage.artifact_limit,
        byte_limit: usage.byte_limit,
        warning_ratio: usage.warning_ratio,
        artifact_count: usage.used_artifacts,
        logical_bytes: usage.used_bytes,
        is_warning: artifact_ratio >= usage.warning_ratio || byte_ratio >= usage.warning_ratio,
        is_exceeded: usage.used_artifacts >= usage.artifact_limit
            || usage.used_bytes >= usage.byte_limit,
    }))
}
