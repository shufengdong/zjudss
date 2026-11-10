//! 上传确认与独立 Data/Struct 资源的 HTTP 边界。
//!
//! 所有 handler 要求当前用户，并只接受稳定资源 ID、opaque cursor/token 和受控 DTO。模型与
//! 资源子事务、alias/血缘、不可变 Parquet、替换和删除原子性由 application/repository 层
//! 拥有；本模块只映射请求、响应与统一下载 helper。

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{AppState, CandidateDecision, ReplacementAction};
use crate::auth::CurrentUser;

use super::{ApiError, attachment, query::parse_columns};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmBody {
    #[serde(default)]
    decisions: Vec<CandidateDecision>,
    replacement_action: Option<ReplacementAction>,
    expected_draft: Option<crate::db::models::DraftRef>,
    client_operation_id: String,
}

/// `POST /api/upload-batches/{id}/confirm`：确认上传候选的发布决定。
///
/// 请求携带候选决定、可选 replacement action、精确 draft 引用和幂等
/// `clientOperationId`。普通上传保持模型与资源两个独立原子子事务；成功或部分成功返回 200
/// 和冻结 receipt，并同步 durable 批次卡片。相同操作不同 payload 返回 409
/// `idempotency_conflict`，跨用户与不存在批次统一返回 404。
pub async fn confirm_upload(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<Value>, ApiError> {
    let confirmation = state
        .upload_confirmation_service
        .confirm(
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

/// `GET /api/sessions/{id}/data-resources`：分页列出当前用户会话的存活资源。
///
/// 接受 opaque `cursor` 和可选 `limit`（缺省 50）；成功返回 200 和下一页游标，不返回服务器
/// 路径。跨用户与不存在会话统一返回 404。
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

/// `GET /api/data-resources/{id}`：读取当前用户资源元数据。
///
/// 成功返回 200、稳定 ID、schema/血缘和当前可用状态；读取不增加 artifact 引用，跨用户、
/// 已删除与不存在资源统一返回 404。
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

/// `PATCH /api/data-resources/{id}`：修改资源显示名或会话内 alias。
///
/// 成功返回 200 和更新后的资源；显式 alias 冲突返回 409 `resource_alias_conflict`，不会修改
/// immutable 数据内容。跨用户与不存在资源统一返回 404。
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

/// `GET /api/data-resources/{id}/rows`：读取资源的 cursor 分页与受控列投影。
///
/// `columns` 保留逗号分段的原始顺序、空白和重复值，空段忽略；service 校验列、limit 和
/// opaque cursor。成功返回 200 且只读，跨用户与不存在资源统一返回 404。
pub async fn rows(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<RowsQuery>,
) -> Result<Json<crate::application::DataResourceRows>, ApiError> {
    let columns = parse_columns(query.columns.as_deref());
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

/// `GET /api/data-resources/{id}/download`：下载资源原文件或规范数据。
///
/// `kind` 缺省为 `normalized`，仅接受 `original|normalized`；成功返回 200 attachment，规范
/// 数据保持不可变 Parquet。无原文件或非法 kind 返回稳定资源错误，响应不包含服务器路径；
/// 跨用户与不存在资源统一返回 404。
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
    attachment::response(download.file, download.media_type, download.file_name).await
}

/// `POST /api/data-resources/{id}/deletion-preview`：生成资源删除影响与短期 token。
///
/// 成功返回 200，统计未确认配置、冻结工作流及派生血缘引用，并持久化当前影响快照；不删除
/// 资源。跨用户与不存在资源统一返回 404。
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

/// `DELETE /api/data-resources/{id}?impactToken=...`：确认删除当前用户资源。
///
/// token 必须来自当前未过期的影响预览。成功返回 200 和 tombstone 摘要，隐藏普通读取并释放
/// 直接 artifact 引用；重复确认保持幂等。影响变化或 token 无效返回稳定 409，跨用户与不
/// 存在资源统一返回 404。
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

/// `POST /api/data-resources/{id}/structured-replacement-previews`：预览 Struct 覆盖。
///
/// 请求携带精确 content hash、schema ref 和受控值；成功校验并 staging 不可变候选，返回
/// 200 和短期 preview token，但不修改或发布资源。来源漂移返回 409 `resource_stale`，跨用户
/// 与不存在资源统一返回 404。
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

/// `POST /api/data-resource-replacement-previews/{id}/confirm`：确认 Struct 覆盖预览。
///
/// 仅接受 `Replace`、`SaveAsNew` 或 `Cancel` 受控动作；成功返回 200 和动作结果，发布时创建
/// 新的不可变资源/血缘而不改写既有 artifact。过期或来源漂移预览返回稳定冲突，跨用户与
/// 不存在预览统一返回 404。
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
