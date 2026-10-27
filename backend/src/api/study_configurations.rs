use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::application::{
    AppState, BindStudyResourceRequest, ConfirmStudyRequest, EventWriter, ManagedWorkflowView,
    SelectFacadeRequest, SelectStudyResultsRequest, StudyConfigCard, StudyConfigurationView,
    UpdateStudyStructRequest,
};
use crate::auth::CurrentUser;

use super::ApiError;

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state.study_configuration_service.get(&user, id).await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn select_facade(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectFacadeRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .select_facade(&user, id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn bind_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<BindStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .bind_resource(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn map_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<crate::application::MapStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .map_resource(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn update_struct(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<UpdateStudyStructRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .update_struct(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn select_results(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectStudyResultsRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .select_results(&user, id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

pub async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<ConfirmStudyRequest>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(
        state
            .managed_workflow_service
            .confirm_configuration_and_start(&user, id, body)
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
    let view = state
        .study_configuration_service
        .cancel(&user, id, query.expected_revision)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

async fn publish(
    state: &AppState,
    user: &CurrentUser,
    view: &StudyConfigurationView,
) -> Result<(), ApiError> {
    let action_card =
        serde_json::to_value(view).map_err(|error| ApiError::internal(error.to_string()))?;
    EventWriter::new(
        user.id.clone(),
        view.session_id,
        state.message_repo.clone(),
        state.events.clone(),
    )
    .study_config(StudyConfigCard {
        configuration_id: view.id.to_string(),
        revision: view.revision,
        state: action_card["status"]
            .as_str()
            .unwrap_or("collecting_inputs")
            .to_string(),
        action_card,
    })
    .await?;
    Ok(())
}
