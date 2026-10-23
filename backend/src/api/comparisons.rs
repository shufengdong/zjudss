use super::ApiError;
use crate::{
    application::{AppState, ms_to_iso},
    auth::CurrentUser,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;
use serde_json::Value;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonDto {
    id: String,
    session_id: String,
    baseline_task_id: String,
    candidate_task_id: String,
    compatibility: Value,
    analysis_run_id: String,
    created_at: String,
    datasets: Vec<crate::application::datasets::DatasetMetadata>,
}
pub async fn get(
    State(s): State<AppState>,
    p: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ComparisonDto>, ApiError> {
    let (c, compatibility, _) = s.comparison_service.find(&p, id).await?;
    let datasets = s
        .dataset_service
        .list_analysis(&p, c.analysis_run_id)
        .await?;
    Ok(Json(ComparisonDto {
        id: c.id.to_string(),
        session_id: c.session_id.to_string(),
        baseline_task_id: c.baseline_task_id.to_string(),
        candidate_task_id: c.candidate_task_id.to_string(),
        compatibility,
        analysis_run_id: c.analysis_run_id.to_string(),
        created_at: ms_to_iso(c.created_at_ms),
        datasets,
    }))
}
