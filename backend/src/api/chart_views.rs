//! 受控图表 capability、预览和不可变保存视图的 HTTP 边界。
//!
//! 所有 handler 要求当前用户，并只接受稳定 dataset/view ID、受控 [`ChartConfig`](crate::application::charting::ChartConfig)
//! 与 opaque cursor。编译器输出稳定 `chart-model-v1` [`ChartModel`](crate::application::charting::ChartModel)，
//! 不是任意 ECharts option；来源解析、预算、Plot Data 物化、幂等和 tombstone 由 chart service
//! 拥有。

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
    request.map(|Json(value)| value).map_err(|_| {
        ApiError::public(
            StatusCode::BAD_REQUEST,
            "chart_config_invalid",
            "图表请求格式无效",
        )
    })
}

fn list_query(
    query: Result<Query<ChartViewListQuery>, QueryRejection>,
) -> Result<ChartViewListQuery, ApiError> {
    query.map(|Query(value)| value).map_err(|_| {
        ApiError::public(
            StatusCode::BAD_REQUEST,
            "chart_cursor_invalid",
            "图表分页参数无效",
        )
    })
}

/// `GET /api/datasets/{id}/chart-capabilities`：发现结果可用的受控图表字段。
///
/// 成功返回 200、来源 schema/hash、标量/List 分量和服务端预算；扫描有硬上限且不持久化视图。
/// 非数字或超限诊断使用稳定图表错误，跨用户、已删除与不存在结果统一返回 404。
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

/// `POST /api/datasets/{id}/chart-previews`：编译非持久化图表预览。
///
/// 请求只接受六类受控配置；成功返回 200 和 `chart-model-v1` `ChartModel`，不是任意 ECharts
/// JSON，也不创建数据库行或 artifact。非法 JSON/config 返回 400 稳定图表错误，跨用户与
/// 不存在来源统一返回 404。
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

/// `POST /api/datasets/{id}/views`：保存不可变图表及物化 Plot Data。
///
/// 请求携带 UUID `clientOperationId`、名称和受控配置；首次原子发布返回 201，相同规范请求
/// 重放返回 200 且 `replayed=true`。同一操作不同 payload 返回 409
/// `chart_operation_conflict`；成功冻结来源快照，因此来源后来删除仍可读取视图。
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

/// `GET /api/dataset-views/{id}`：读取已保存图表的冻结元数据。
///
/// 成功返回 200、规范配置、来源快照、诊断和 `sourceDeleted` 状态；不重新编译或读取 live
/// 来源。跨用户、已删除与不存在视图统一返回 404。
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

/// `GET /api/dataset-views/{id}/data`：读取已物化图表数据模型。
///
/// 成功返回 200 和冻结 `chart-model-v1` `ChartModel`，不是 ECharts option；数据来自视图自有
/// Plot Data，来源结果 tombstone 后仍可读取。跨用户、已删除与不存在视图统一返回 404。
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

/// `DELETE /api/dataset-views/{id}`：tombstone 已保存图表。
///
/// 成功返回 200 和删除摘要，释放视图自有 Plot Data artifact，但不恢复或删除来源结果，也不
/// 影响其他视图。跨用户、已删除与不存在视图统一返回 404。
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

/// `GET /api/sessions/{id}/dataset-views`：分页列出会话内存活图表。
///
/// 接受 opaque `cursor` 和可选 `limit`（缺省 20）；成功返回 200，保留来源已删除但自身仍
/// 存活的视图。非法分页返回 400 `chart_cursor_invalid`，跨用户与不存在会话统一返回 404。
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
