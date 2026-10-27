use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Response, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::application::{AppState, CandidateDecision, ReplacementAction};
use crate::auth::CurrentUser;

use super::ApiError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmBody {
    #[serde(default)]
    decisions: Vec<CandidateDecision>,
    replacement_action: Option<ReplacementAction>,
    expected_draft: Option<crate::db::models::DraftRef>,
    client_operation_id: String,
}

pub async fn confirm_upload(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<Value>, ApiError> {
    let confirmation = state
        .data_resource_service
        .confirm_upload(
            &user,
            id,
            body.decisions,
            body.replacement_action,
            body.expected_draft,
            &body.client_operation_id,
        )
        .await?;
    let batch = state.upload_batch_service.get(&user, id).await?;
    super::uploads::publish_upload(&state, &user, &batch).await?;
    Ok(Json(json!({"uploadConfirmation": confirmation})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    cursor: Option<String>,
    limit: Option<u16>,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Query(query): Query<ListQuery>,
) -> Result<Json<crate::application::DataResourcePage>, ApiError> {
    Ok(Json(
        state
            .data_resource_service
            .list(&user, session_id, query.cursor, query.limit.unwrap_or(50))
            .await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::DataResourceView>, ApiError> {
    Ok(Json(state.data_resource_service.get(&user, &id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    display_name: Option<String>,
    alias: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<Json<crate::application::DataResourceView>, ApiError> {
    Ok(Json(
        state
            .data_resource_service
            .update(&user, &id, body.display_name, body.alias)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct RowsQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    columns: Option<String>,
}

pub async fn rows(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<RowsQuery>,
) -> Result<Json<crate::application::DataResourceRows>, ApiError> {
    let columns = query
        .columns
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    Ok(Json(
        state
            .data_resource_service
            .rows(&user, &id, query.limit, query.cursor.as_deref(), columns)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    kind: Option<String>,
}

pub async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> Result<Response<Body>, ApiError> {
    let original = match query.kind.as_deref().unwrap_or("normalized") {
        "original" => true,
        "normalized" => false,
        _ => {
            return Err(crate::application::DataResourceError::Invalid(
                "data_resource_download_invalid",
                "kind 只支持 original 或 normalized".into(),
            )
            .into());
        }
    };
    let download = state
        .data_resource_service
        .download(&user, &id, original)
        .await?;
    let mut file = tokio::fs::File::from_std(download.file);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, download.media_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", download.file_name),
        )
        .body(Body::from(bytes))
        .map_err(|error| ApiError::internal(error.to_string()))
}

pub async fn deletion_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::DataResourceDeletionPreview>, ApiError> {
    Ok(Json(
        state
            .data_resource_service
            .preview_deletion(&user, &id)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    impact_token: String,
}

pub async fn delete_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<crate::application::DeletedDataResource>, ApiError> {
    Ok(Json(
        state
            .data_resource_service
            .delete(&user, &id, &query.impact_token)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredPreviewBody {
    expected_content_hash: String,
    schema_ref: String,
    values: Value,
}

pub async fn structured_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<StructuredPreviewBody>,
) -> Result<Json<crate::application::StructuredReplacementPreview>, ApiError> {
    Ok(Json(
        state
            .data_resource_service
            .preview_structured_replacement(
                &user,
                &id,
                &body.expected_content_hash,
                &body.schema_ref,
                body.values,
            )
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct StructuredConfirmBody {
    action: ReplacementAction,
}

pub async fn confirm_structured_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(token): Path<String>,
    Json(body): Json<StructuredConfirmBody>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"resource":state.data_resource_service.confirm_structured_replacement(&user, &token, body.action).await?}),
    ))
}
