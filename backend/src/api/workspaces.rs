//! 模型工作区、草稿发布、历史版本激活与删除的 HTTP 边界。
//!
//! 所有 handler 要求当前用户，并把会话/版本稳定 ID 与精确 revision、幂等键或 impact token
//! 交给 version service。草稿/版本不可变发布、active 指针、artifact 引用和删除事务均不在
//! 本模块实现。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{ActivateVersionRequest, AppState, PublishDraftRequest, ms_to_iso};
use crate::auth::CurrentUser;
use crate::db::repositories::{DraftSnapshot, VersionSnapshot, WorkspaceSnapshot};

use super::ApiError;

/// `GET /api/sessions/{id}/workspace`：读取当前用户会话的模型工作区。
///
/// 成功返回 200、active 版本和当前 draft 快照；首次访问可按需建立空 workspace 容器，但不
/// 创建草稿或激活版本。跨用户与不存在会话统一返回 404。
pub async fn workspace(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(workspace_json(
        state.version_service.workspace(&user, session_id).await?,
    )))
}

/// `GET /api/sessions/{id}/versions`：列出会话的不可变模型版本。
///
/// 成功返回 200 和当前用户可读的历史版本；首次访问可按需建立空 workspace 容器，但不移动
/// active 指针或增加 artifact 引用。跨用户与不存在会话统一返回 404。
pub async fn versions(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let versions = state.version_service.versions(&user, session_id).await?;
    Ok(Json(
        json!({"versions": versions.into_iter().map(version_json).collect::<Vec<_>>() }),
    ))
}

/// `POST /api/sessions/{id}/versions`：显式发布精确草稿 revision 并自动激活。
///
/// 请求包含 expected draft、幂等 client publish ID 与可选描述；首次创建返回 201，内容去重或
/// 同一 draft revision/client ID 重放返回 200。revision 漂移返回 409
/// `draft_revision_conflict`；失败不清空或部分发布草稿。
pub async fn publish(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Json(request): Json<PublishDraftRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let published = state
        .version_service
        .publish(&user, session_id, request)
        .await?;
    let status = if published.replayed || published.deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(
            json!({"version":version_json(published.version),"deduplicated":published.deduplicated,"replayed":published.replayed}),
        ),
    ))
}

/// `GET /api/versions/{id}`：读取当前用户的不可变版本详情。
///
/// 成功返回 200、冻结描述、来源摘要和表清单；读取不激活版本，跨用户、已删除与不存在版本
/// 统一返回 404。
pub async fn version(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(version_json(
        state.version_service.version(&user, version_id).await?,
    )))
}

/// `POST /api/versions/{id}/activate`：把历史版本设为当前 active。
///
/// 成功返回 200 和更新后的工作区；存在 draft 时请求必须携带精确 draft 引用并明确允许放弃，
/// 否则返回稳定 409。激活只移动指针，不改写不可变版本；跨用户与不存在版本统一返回 404。
pub async fn activate(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
    Json(request): Json<ActivateVersionRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(workspace_json(
        state
            .version_service
            .activate(&user, version_id, request)
            .await?,
    )))
}

/// `POST /api/versions/{id}/deletion-preview`：生成版本删除影响与短期 token。
///
/// 成功返回 200 并冻结当前消费者/影响摘要，不删除版本。active 或跨用户/不存在版本按 version
/// service 的稳定冲突/404 语义返回。
pub async fn deletion_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(
            state
                .version_service
                .preview_deletion(&user, version_id)
                .await?,
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    impact_token: String,
}

/// `DELETE /api/versions/{id}?impactToken=...`：确认删除不可变历史版本。
///
/// 仅非 active、无忙碌消费者且影响未变化的版本可删除；成功原子 tombstone 并释放直接
/// artifact 引用，返回 204。无效/过期 token、影响变化或 busy 返回稳定 409，跨用户与不存在
/// 版本统一返回 404。
pub async fn delete_version(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(version_id): Path<i32>,
    Query(query): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    state
        .version_service
        .delete(&user, version_id, query.impact_token)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn workspace_json(snapshot: WorkspaceSnapshot) -> Value {
    json!({
        "id": snapshot.workspace.id.to_string(),
        "sessionId": snapshot.workspace.session_id.to_string(),
        "active": snapshot.active.map(version_json),
        "draft": snapshot.draft.map(draft_json),
        "activeModelImports": []
    })
}

fn draft_json(snapshot: DraftSnapshot) -> Value {
    json!({
        "id": snapshot.draft.id.to_string(),
        "workspaceId": snapshot.draft.workspace_id.to_string(),
        "baseVersionId": snapshot.draft.base_version_id.map(|id| id.to_string()),
        "revision": snapshot.draft.revision,
        "diagnostics": serde_json::from_str::<Value>(&snapshot.draft.diagnostics_json).unwrap_or_else(|_| json!({})),
        "tables": snapshot.dataframes.into_iter().map(|table| json!({
            "id":table.id.to_string(),"tableName":table.table_name,"artifactId":table.artifact_id.to_string(),
            "schema":serde_json::from_str::<Value>(&table.schema_json).unwrap_or_else(|_| json!([])),
            "rowCount":table.row_count,"contentHash":table.content_hash
        })).collect::<Vec<_>>(),
        "createdAt":ms_to_iso(snapshot.draft.created_at_ms),"updatedAt":ms_to_iso(snapshot.draft.updated_at_ms)
    })
}

fn version_json(snapshot: VersionSnapshot) -> Value {
    json!({
        "id":snapshot.version.id.to_string(),"workspaceId":snapshot.version.workspace_id.to_string(),
        "description":snapshot.version.description,
        "contentHash":snapshot.version.content_hash,
        "sourceSummary":serde_json::from_str::<Value>(&snapshot.version.source_summary_json).unwrap_or_else(|_| json!({})),
        "engineContractVersion":snapshot.version.engine_contract_version,
        "tables":snapshot.dataframes.into_iter().map(|table| json!({
            "id":table.id.to_string(),"tableName":table.table_name,"artifactId":table.artifact_id.to_string(),
            "schema":serde_json::from_str::<Value>(&table.schema_json).unwrap_or_else(|_| json!([])),
            "rowCount":table.row_count,"contentHash":table.content_hash
        })).collect::<Vec<_>>(),
        "createdAt":ms_to_iso(snapshot.version.created_at_ms)
    })
}
