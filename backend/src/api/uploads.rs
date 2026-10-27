use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{
    AppState, CandidateDecision, ParseSettings, UploadBatchError, UploadFile, UploadSourceKind,
};
use crate::auth::CurrentUser;
use crate::storage::StageLimits;

use super::ApiError;

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
    Ok((StatusCode::ACCEPTED, Json(json!({"uploadBatch":view}))))
}

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"uploadBatch":state.upload_batch_service.get(&user,id).await?}),
    ))
}

pub async fn parse_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(settings): Json<ParseSettings>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"uploadBatch":state.upload_batch_service.update_parse_settings(&user,id,settings).await?}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateBody {
    decisions: Vec<CandidateDecision>,
}
pub async fn candidates(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<CandidateBody>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"uploadBatch":state.upload_batch_service.update_candidates(&user,id,body.decisions).await?}),
    ))
}
pub async fn discard(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.upload_batch_service.discard(&user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

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
