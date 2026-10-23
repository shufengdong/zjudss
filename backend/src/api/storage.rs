use axum::Json;
use axum::extract::State;

use crate::application::{AppState, StorageUsageSummary};
use crate::auth::CurrentUser;

use super::ApiError;

pub async fn usage(
    State(state): State<AppState>,
    principal: CurrentUser,
) -> Result<Json<StorageUsageSummary>, ApiError> {
    state
        .artifact_service
        .usage_for(principal.storage_scope_id)
        .await
        .map(Json)
        .map_err(Into::into)
}
