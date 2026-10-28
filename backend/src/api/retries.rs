use axum::Json;
use axum::extract::{Path, State};

use crate::application::{AppState, ConfirmRetryRequest, ManagedWorkflowView};
use crate::auth::CurrentUser;

use super::ApiError;

pub async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ConfirmRetryRequest>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(
        state
            .managed_workflow_service
            .confirm_retry(&user, &id, body)
            .await?,
    ))
}
