use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::application::AppState;
use crate::auth::CurrentPrincipal;

use super::ApiError;

pub async fn get(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let ingestion = state.ingestion_service.get(&principal, id).await?;
    Ok(Json(json!({ "ingestion": ingestion })))
}
