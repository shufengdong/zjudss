//! 统一上传批次的接收、恢复与编辑 HTTP 边界。
//!
//! multipart handler 只解析受控字段并把字节流写入 staging；单文件、批次和 artifact 配额由
//! storage/application 层强制执行。批次解析、候选状态机、替换规则和发布事务不在本模块；
//! 每次可见状态更新会同步 durable `upload-batch` 卡片。

use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{
    AppState, CandidateDecision, EventDiagnostic, EventWriter, ParseSettings, UploadBatchCard,
    UploadBatchError, UploadBatchView, UploadCandidateCard, UploadFile, UploadSourceKind,
};
use crate::auth::CurrentUser;
use crate::storage::StageLimits;

use super::ApiError;

/// `POST /api/sessions/{id}/upload-batches`：接收当前用户会话的一批上传文件。
///
/// 路由关闭 Axum 默认 body limit 以支持流式 multipart，但 application 配置仍强制单文件与
/// 批次容量。请求包含 `clientBatchId`、可选 `relativePaths` 和 `file.<clientFileId>`；成功
/// staging 并启动解析后返回 202 和可恢复批次。幂等冲突、格式/归档限制和配额错误保留稳定
/// code，跨用户与不存在会话统一返回 404。
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    accept_multipart(state, user, session_id, None, multipart).await
}

async fn accept_multipart(
    state: AppState,
    user: CurrentUser,
    session_id: i32,
    replacement_resource_id: Option<String>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut client_batch_id = None;
    let mut relative_paths = std::collections::HashMap::<String, String>::new();
    let mut pending = Vec::new();
    let mut total = 0_u64;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "clientBatchId" {
            client_batch_id = Some(
                field
                    .text()
                    .await
                    .map_err(|e| ApiError::internal(e.to_string()))?,
            );
            continue;
        }
        if name == "relativePaths" {
            relative_paths = serde_json::from_str(
                &field
                    .text()
                    .await
                    .map_err(|e| ApiError::internal(e.to_string()))?,
            )
            .map_err(|_| {
                UploadBatchError::Invalid(
                    "upload_invalid".into(),
                    "relativePaths 必须是 JSON 对象".into(),
                )
            })?;
            continue;
        }
        let Some(client_file_id) = name.strip_prefix("file.") else {
            continue;
        };
        let file_name = field
            .file_name()
            .map(|v| v.replace(['/', '\\'], "_"))
            .unwrap_or_else(|| "upload".into());
        let media_type = field.content_type().map(str::to_string);
        let mut writer = state
            .artifact_service
            .store()
            .begin_stage(StageLimits {
                max_bytes: state.upload_single_file_limit_bytes,
            })
            .map_err(crate::application::ArtifactError::from)?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
        {
            total = total.saturating_add(chunk.len() as u64);
            if total > state.upload_batch_limit_bytes {
                return Err(UploadBatchError::Invalid(
                    "upload_batch_too_large".into(),
                    "上传批次超过限制".into(),
                )
                .into());
            }
            writer
                .write_chunk(&chunk)
                .map_err(crate::application::ArtifactError::from)?;
        }
        pending.push((
            client_file_id.to_string(),
            file_name,
            media_type,
            writer
                .finish()
                .map_err(crate::application::ArtifactError::from)?,
        ));
    }
    let files = pending
        .into_iter()
        .map(|(client_file_id, file_name, media_type, staged)| {
            let relative_path = relative_paths
                .remove(&client_file_id)
                .map(|path| crate::ingestion::safe_relative(&path))
                .transpose()
                .map_err(|error| {
                    UploadBatchError::Invalid(error.code().into(), error.to_string())
                })?;
            Ok(UploadFile {
                client_file_id,
                file_name,
                relative_path,
                media_type,
                staged,
            })
        })
        .collect::<Result<Vec<_>, UploadBatchError>>()?;
    let client_batch_id = client_batch_id.ok_or_else(|| {
        UploadBatchError::Invalid("upload_invalid".into(), "缺少 clientBatchId".into())
    })?;
    let view = if let Some(resource_id) = replacement_resource_id {
        state
            .upload_batch_service
            .accept_replacement(&user, session_id, client_batch_id, resource_id, files)
            .await?
    } else {
        state
            .upload_batch_service
            .accept(
                &user,
                session_id,
                client_batch_id,
                UploadSourceKind::Upload,
                files,
            )
            .await?
    };
    publish_upload(&state, &user, &view).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"uploadBatch":view}))))
}

/// `GET /api/upload-batches/{id}`：读取当前用户上传批次的权威状态。
///
/// 成功返回 200，并幂等同步对应 durable 上传卡片以修复先前消息写入中断；不会重新提交发布
/// 事务。跨用户与不存在批次统一返回 404。
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let view = state.upload_batch_service.get(&user, id).await?;
    publish_upload(&state, &user, &view).await?;
    Ok(Json(json!({"uploadBatch":view})))
}

/// `PUT /api/upload-batches/{id}/parse-settings`：更新受控解析设置并重新解析批次。
///
/// 仅当前用户可修改等待解析设置的批次；成功返回 200 和新 revision 的批次，并同步 durable
/// 卡片。终态、过期或正在确认的批次返回稳定状态冲突，不会破坏已保存候选。
pub async fn parse_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(settings): Json<ParseSettings>,
) -> Result<Json<Value>, ApiError> {
    let view = state
        .upload_batch_service
        .update_parse_settings(&user, id, settings)
        .await?;
    publish_upload(&state, &user, &view).await?;
    Ok(Json(json!({"uploadBatch":view})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateBody {
    decisions: Vec<CandidateDecision>,
}

/// `PUT /api/upload-batches/{id}/candidates`：保存上传候选的用途决定。
///
/// 决定仅限模型表、普通资源或跳过等受控枚举；成功返回 200、更新后的批次并同步 durable
/// 卡片。已冻结、终态或正在确认的批次拒绝修改，跨用户与不存在批次统一返回 404。
pub async fn candidates(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<CandidateBody>,
) -> Result<Json<Value>, ApiError> {
    let view = state
        .upload_batch_service
        .update_candidates(&user, id, body.decisions)
        .await?;
    publish_upload(&state, &user, &view).await?;
    Ok(Json(json!({"uploadBatch":view})))
}

pub(super) async fn publish_upload(
    state: &AppState,
    user: &CurrentUser,
    view: &UploadBatchView,
) -> Result<(), ApiError> {
    let state_name = serde_json::to_value(view.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "processing".into());
    let child_state = |value| {
        serde_json::to_value(value)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "not_applicable".into())
    };
    let diagnostic = view
        .diagnostics
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| {
            Some(EventDiagnostic {
                code: item.get("code")?.as_str()?.to_string(),
                message: item.get("message")?.as_str()?.to_string(),
            })
        });
    EventWriter::new(
        user.id.clone(),
        view.session_id
            .parse()
            .map_err(|_| ApiError::internal("invalid upload session"))?,
        state.message_repo.clone(),
        state.events.clone(),
    )
    .upload_batch(UploadBatchCard {
        batch_id: view.id.clone(),
        batch_revision: view.revision,
        state: state_name,
        candidates: view
            .candidates
            .iter()
            .map(|candidate| UploadCandidateCard {
                id: candidate.id.clone(),
                name: candidate.display_name.clone(),
                state: if candidate.selected {
                    "selected".into()
                } else {
                    "skipped".into()
                },
            })
            .collect(),
        model_transaction_state: child_state(view.model_publish_status),
        resource_transaction_state: child_state(view.resource_publish_status),
        diagnostic,
    })
    .await?;
    Ok(())
}

/// `DELETE /api/upload-batches/{id}`：放弃当前用户的未完成上传批次。
///
/// 成功幂等释放仍由批次持有的 staging/artifact 引用、同步终态卡片并返回 204；确认正在执行
/// 时返回 409 `upload_state_invalid`，跨用户与不存在批次统一返回 404。
pub async fn discard(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.upload_batch_service.discard(&user, id).await?;
    let view = state.upload_batch_service.get(&user, id).await?;
    publish_upload(&state, &user, &view).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/data-resources/{id}/replacement-batches`：为现有资源接收替换上传。
///
/// 路由与普通上传一样关闭 Axum 默认 body limit，但仍执行 application 容量限制。当前用户
/// 必须拥有资源；成功仅创建并解析替换候选批次，返回 202，不会在确认前修改原资源。跨用户
/// 与不存在资源统一返回 404。
pub async fn replacement(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let session_id = state
        .upload_batch_service
        .replacement_session(&user, &id)
        .await?;
    accept_multipart(state, user, session_id, Some(id), multipart).await
}
