use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::ApiError;
use crate::application::AppState;
use crate::application::charting::{ChartPreviewRequest, CreateChartViewRequest};
use crate::auth::CurrentUser;

#[derive(Debug, Deserialize)]
pub struct ChartViewListQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

fn public_id(value: &str) -> Result<i32, ApiError> {
    value.parse().map_err(|_| ApiError::not_found("资源不存在"))
}

fn json_request<T>(request: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    request.map(|Json(value)| value).map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "图表请求格式无效".into(),
        code: "chart_config_invalid",
        details: None,
    })
}

fn list_query(
    query: Result<Query<ChartViewListQuery>, QueryRejection>,
) -> Result<ChartViewListQuery, ApiError> {
    query.map(|Query(value)| value).map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "图表分页参数无效".into(),
        code: "chart_cursor_invalid",
        details: None,
    })
}

pub async fn capabilities(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::charting::ChartCapabilities>, ApiError> {
    Ok(Json(
        state
            .chart_service
            .capabilities(&principal, public_id(&id)?)
            .await?,
    ))
}

pub async fn preview(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
    request: Result<Json<ChartPreviewRequest>, JsonRejection>,
) -> Result<Json<crate::application::charting::ChartModel>, ApiError> {
    let request = json_request(request)?;
    Ok(Json(
        state
            .chart_service
            .preview(&principal, public_id(&id)?, request.config)
            .await?,
    ))
}

pub async fn save(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
    request: Result<Json<CreateChartViewRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let request = json_request(request)?;
    let response = state
        .chart_service
        .save(&principal, public_id(&id)?, request)
        .await?;
    let status = if response.replayed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
}

pub async fn metadata(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::charting::ChartViewMetadata>, ApiError> {
    Ok(Json(
        state
            .chart_service
            .metadata(&principal, public_id(&id)?)
            .await?,
    ))
}

pub async fn data(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::charting::ChartModel>, ApiError> {
    Ok(Json(
        state
            .chart_service
            .data(&principal, public_id(&id)?)
            .await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<crate::application::charting::DeleteChartViewResponse>, ApiError> {
    Ok(Json(
        state
            .chart_service
            .delete(&principal, public_id(&id)?)
            .await?,
    ))
}

pub async fn list(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(id): Path<String>,
    query: Result<Query<ChartViewListQuery>, QueryRejection>,
) -> Result<Json<crate::application::charting::ChartViewPage>, ApiError> {
    let query = list_query(query)?;
    Ok(Json(
        state
            .chart_service
            .list(
                &principal,
                public_id(&id)?,
                query.cursor.as_deref(),
                query.limit.unwrap_or(20),
            )
            .await?,
    ))
}
