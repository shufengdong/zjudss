//! 已发布模型版本与精确草稿 revision 的 DataFrame 只读 API。
//!
//! 所有路由要求当前用户；已发布表由稳定表 ID 定位，草稿表还必须携带精确 draft ID/revision。
//! handler 只映射 metadata/page/download，请求不会激活版本、推进草稿或增加 artifact 引用。

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::Response;
use serde::Deserialize;

use super::{ApiError, attachment, query::parse_columns};
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

/// `GET /api/model-dataframes/{id}`：读取不可变版本表元数据。
///
/// 当前用户拥有对应版本时返回 200、schema/hash/行数等只读信息；跨用户、已删除或不存在表
/// 统一返回 404。
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

/// `GET /api/model-dataframes/{id}/rows`：分页读取不可变版本表。
///
/// 接受受控 `limit`、资源绑定 opaque cursor 和逗号列投影；投影保留现有空白、顺序与重复
/// 语义。成功返回 200，非法分页返回稳定 400，读取不改变模型生命周期。
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

/// `GET /api/model-dataframes/{id}/download`：下载不可变版本表 Parquet。
///
/// 当前用户拥有表时返回 200 attachment；下载使用已授权文件句柄，不暴露服务器路径，也不
/// 增加 artifact 引用。跨用户、已删除或不存在表统一返回 404。
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

/// `GET /api/draft-dataframes/{id}`：读取精确草稿 revision 的表元数据。
///
/// 查询必须携带 `draftId` 和 `expectedDraftRevision`；成功返回 200。任一表变化后旧 revision
/// 返回 409 `draft_revision_conflict`，跨用户或不存在表统一返回 404。
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

/// `GET /api/draft-dataframes/{id}/rows`：分页读取精确草稿 revision 的表。
///
/// 除精确 draft ID/revision 外，接受受控 `limit`、资源绑定 cursor 和逗号列投影；成功返回
/// 200。旧 revision 返回 409，非法分页返回稳定 400，读取不会推进草稿。
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

/// `GET /api/draft-dataframes/{id}/download`：下载精确草稿 revision 的 Parquet。
///
/// 查询必须匹配当前 draft ID/revision；成功返回 200 attachment。revision 漂移返回 409
/// `draft_revision_conflict`，响应不含物理路径，跨用户或不存在表统一返回 404。
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

async fn download_response(
    file: crate::application::model_dataframes::ModelDataFrameDownload,
) -> Result<Response<Body>, ApiError> {
    attachment::response(file.file, file.media_type, file.file_name).await
}
