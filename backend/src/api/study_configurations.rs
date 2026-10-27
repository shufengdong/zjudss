use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::application::{
    AppState, BindStudyResourceRequest, SelectFacadeRequest, SelectStudyResultsRequest,
    StudyConfigurationView, UpdateStudyStructRequest,
};
use crate::auth::CurrentUser;

use super::ApiError;

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state.study_configuration_service.get(&user, id).await?,
    ))
}

pub async fn select_facade(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectFacadeRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .select_facade(&user, id, body)
            .await?,
    ))
}

pub async fn bind_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<BindStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .bind_resource(&user, id, &input_id, body)
            .await?,
    ))
}

pub async fn map_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<crate::application::MapStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .map_resource(&user, id, &input_id, body)
            .await?,
    ))
}

pub async fn update_struct(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<UpdateStudyStructRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .update_struct(&user, id, &input_id, body)
            .await?,
    ))
}

pub async fn select_results(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectStudyResultsRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .select_results(&user, id, body)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQuery {
    expected_revision: i64,
}

pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<CancelQuery>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    Ok(Json(
        state
            .study_configuration_service
            .cancel(&user, id, query.expected_revision)
            .await?,
    ))
}
