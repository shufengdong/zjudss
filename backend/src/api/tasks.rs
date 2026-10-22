//! 任务结果 handler：曲线数据（JSON）与报告文件（二进制下载）。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;

use crate::application::AppState;
use crate::auth::CurrentPrincipal;

use super::ApiError;

/// `GET /api/tasks/{id}/curves` → 200 `CurveData`；无曲线 → 404。
pub async fn curves(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(task_id): Path<i32>,
) -> Result<Json<crate::adapters::CurveData>, ApiError> {
    state
        .task_repo
        .find(&principal.access, task_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    match state.engine.read_curves(task_id).await? {
        Some(data) => Ok(Json(data)),
        None => Err(ApiError::not_found("该任务没有曲线数据")),
    }
}

/// `GET /api/tasks/{id}/report` → 200 二进制下载；无报告 → 404。
pub async fn report(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(task_id): Path<i32>,
) -> Result<axum::response::Response, ApiError> {
    state
        .task_repo
        .find(&principal.access, task_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    match state.engine.read_report(task_id).await? {
        Some(file) => Ok((
            [
                (header::CONTENT_TYPE, file.content_type),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", file.filename),
                ),
            ],
            file.bytes,
        )
            .into_response()),
        None => Err(ApiError::not_found("该任务没有报告")),
    }
}
