use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Response, StatusCode, header};
use serde::Deserialize;

use super::ApiError;
use crate::application::AppState;
use crate::auth::CurrentPrincipal;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    columns: Option<String>,
}

pub async fn metadata(
    State(s): State<AppState>,
    CurrentPrincipal(p): CurrentPrincipal,
    Path(id): Path<i32>,
) -> Result<Json<crate::application::datasets::DatasetMetadata>, ApiError> {
    Ok(Json(s.dataset_service.metadata(&p, id).await?))
}
pub async fn rows(
    State(s): State<AppState>,
    CurrentPrincipal(p): CurrentPrincipal,
    Path(id): Path<i32>,
    Query(q): Query<RowsQuery>,
) -> Result<Json<crate::application::datasets::DatasetPage>, ApiError> {
    let cols = q
        .columns
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    Ok(Json(
        s.dataset_service
            .page(&p, id, q.limit, q.cursor.as_deref(), &cols)
            .await?,
    ))
}
pub async fn download(
    State(s): State<AppState>,
    CurrentPrincipal(p): CurrentPrincipal,
    Path(id): Path<i32>,
) -> Result<Response<Body>, ApiError> {
    let f = s.dataset_service.download(&p, id).await?;
    let bytes = tokio::fs::read(f.path)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, f.media_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", f.file_name),
        )
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(e.to_string()))
}
