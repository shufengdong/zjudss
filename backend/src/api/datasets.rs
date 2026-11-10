//! 仿真与派生结果数据集的读取和单结果删除 HTTP 边界。
//!
//! 所有 handler 按当前用户解析稳定 dataset ID，并委托 dataset service 保持 live/tombstone、
//! artifact 与派生消费者规则。本模块只提供 metadata、受控分页、不可变下载和明确删除入口。

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::Response;
use serde::Deserialize;

use super::{ApiError, attachment, query::parse_columns};
use crate::application::AppState;
use crate::auth::CurrentUser;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    columns: Option<String>,
}

/// `GET /api/datasets/{id}`：读取当前用户的存活结果元数据。
///
/// 成功返回 200、schema/血缘与不可变内容信息；该读取不改变运行或 artifact，跨用户、已删除
/// 与不存在结果统一返回 404。
pub async fn metadata(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<crate::application::datasets::DatasetMetadata>, ApiError> {
    Ok(Json(state.dataset_service.metadata(&user, id).await?))
}

/// `DELETE /api/datasets/{id}`：删除当前用户的单个存活结果。
///
/// 成功返回 200 和 tombstone 摘要并释放直接 artifact 引用；运行本身及已物化派生血缘保留。
/// 正被分析消费的结果返回 409 `result_busy`，跨用户、已删除与不存在结果统一返回 404。
pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<crate::application::datasets::DeletedDatasetDto>, ApiError> {
    Ok(Json(state.dataset_service.delete(&user, id).await?))
}

/// `GET /api/datasets/{id}/rows`：读取结果的 cursor 分页与受控列投影。
///
/// `columns` 保留逗号分段的原始空白、顺序和重复值，空段忽略；service 校验 limit、列和资源
/// 绑定 cursor。成功返回 200 且只读，非法分页返回 400，跨用户与不存在结果统一返回 404。
pub async fn rows(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<RowsQuery>,
) -> Result<Json<crate::application::datasets::DatasetPage>, ApiError> {
    let columns = parse_columns(query.columns.as_deref());
    Ok(Json(
        state
            .dataset_service
            .page(&user, id, query.limit, query.cursor.as_deref(), &columns)
            .await?,
    ))
}

/// `GET /api/datasets/{id}/download`：下载不可变结果 Parquet。
///
/// 当前用户拥有存活结果时返回 200 attachment；下载使用已授权句柄，不暴露服务器路径，也不
/// 修改运行或 artifact 引用。跨用户、已删除与不存在结果统一返回 404。
pub async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response<Body>, ApiError> {
    let download = state.dataset_service.download(&user, id).await?;
    attachment::response(download.file, download.media_type, download.file_name).await
}
