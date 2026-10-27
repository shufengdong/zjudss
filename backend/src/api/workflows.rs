use axum::Json;
use axum::extract::{Path, State};

use crate::application::{AppState, ManagedWorkflowView};
use crate::auth::CurrentUser;

use super::ApiError;

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(state.managed_workflow_service.get(&user, id).await?))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(
        state.managed_workflow_service.cancel(&user, id).await?,
    ))
}
