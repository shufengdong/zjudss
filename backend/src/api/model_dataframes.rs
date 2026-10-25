use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Response, StatusCode, header};
use serde::Deserialize;
use tokio::io::AsyncReadExt;

use super::ApiError;
use crate::application::{AppState, DraftReadExpectation};
use crate::auth::CurrentUser;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    columns: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftQuery {
    draft_id: i32,
    expected_draft_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRowsQuery {
    draft_id: i32,
    expected_draft_revision: i64,
    limit: Option<usize>,
    cursor: Option<String>,
    columns: Option<String>,
}

pub async fn version_metadata(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<crate::application::model_dataframes::ModelDataFrameMetadata>, ApiError> {
    Ok(Json(
        state
            .model_dataframe_read_service
            .version_metadata(&user, id)
            .await?,
    ))
}

pub async fn version_rows(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<RowsQuery>,
) -> Result<Json<crate::application::model_dataframes::ModelDataFramePage>, ApiError> {
    Ok(Json(
        state
            .model_dataframe_read_service
            .version_page(
                &user,
                id,
                query.limit,
                query.cursor.as_deref(),
                &parse_columns(query.columns.as_deref()),
            )
            .await?,
    ))
}

pub async fn version_download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response<Body>, ApiError> {
    let file = state
        .model_dataframe_read_service
        .version_download(&user, id)
        .await?;
    download_response(file).await
}

pub async fn draft_metadata(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<DraftQuery>,
) -> Result<Json<crate::application::model_dataframes::ModelDataFrameMetadata>, ApiError> {
    Ok(Json(
        state
            .model_dataframe_read_service
            .draft_metadata(&user, id, expectation(&query))
            .await?,
    ))
}

pub async fn draft_rows(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<DraftRowsQuery>,
) -> Result<Json<crate::application::model_dataframes::ModelDataFramePage>, ApiError> {
    Ok(Json(
        state
            .model_dataframe_read_service
            .draft_page(
                &user,
                id,
                DraftReadExpectation {
                    draft_id: query.draft_id,
                    expected_revision: query.expected_draft_revision,
                },
                query.limit,
                query.cursor.as_deref(),
                &parse_columns(query.columns.as_deref()),
            )
            .await?,
    ))
}

pub async fn draft_download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<DraftQuery>,
) -> Result<Response<Body>, ApiError> {
    let file = state
        .model_dataframe_read_service
        .draft_download(&user, id, expectation(&query))
        .await?;
    download_response(file).await
}

fn expectation(query: &DraftQuery) -> DraftReadExpectation {
    DraftReadExpectation {
        draft_id: query.draft_id,
        expected_revision: query.expected_draft_revision,
    }
}

fn parse_columns(columns: Option<&str>) -> Vec<String> {
    columns
        .unwrap_or("")
        .split(',')
        .filter(|column| !column.is_empty())
        .map(str::to_string)
        .collect()
}

async fn download_response(
    file: crate::application::model_dataframes::ModelDataFrameDownload,
) -> Result<Response<Body>, ApiError> {
    let mut source = tokio::fs::File::from_std(file.file);
    let mut bytes = Vec::new();
    source
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, file.media_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file.file_name),
        )
        .body(Body::from(bytes))
        .map_err(|error| ApiError::internal(error.to_string()))
}
