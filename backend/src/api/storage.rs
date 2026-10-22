use axum::Json;
use axum::extract::State;

use crate::application::{AppState, StorageUsageSummary};
use crate::auth::CurrentPrincipal;

use super::ApiError;

pub async fn usage(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
) -> Result<Json<StorageUsageSummary>, ApiError> {
    state
        .artifact_service
        .usage_for(&principal.storage)
        .await
        .map(Json)
        .map_err(Into::into)
}
