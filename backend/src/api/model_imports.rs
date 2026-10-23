use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{AppState, UploadBatch, UploadFile};
use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
use crate::storage::StageLimits;

use super::ApiError;

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut client_import_id = None;
    let mut expected_draft_id = None;
    let mut expected_draft_revision = None;
    let mut relative_paths = std::collections::HashMap::<String, String>::new();
    let mut files = Vec::new();
    let mut batch_bytes = 0_u64;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::internal(format!("读取上传字段失败: {error}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "clientImportId" => {
                client_import_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::internal(error.to_string()))?,
                );
            }
            "expectedDraftId" => {
                expected_draft_id = field.text().await.ok().and_then(|value| value.parse().ok());
            }
            "expectedDraftRevision" => {
                expected_draft_revision =
                    field.text().await.ok().and_then(|value| value.parse().ok());
            }
            "relativePaths" => {
                relative_paths = serde_json::from_str(
                    &field
                        .text()
                        .await
                        .map_err(|error| ApiError::internal(error.to_string()))?,
                )
                .map_err(|_| {
                    crate::application::ModelImportError::Invalid(
                        "relativePaths 必须是 JSON 对象".into(),
                    )
                })?;
            }
            _ => {
                let Some(client_file_id) = name.strip_prefix("file.") else {
                    continue;
                };
                let file_name = field
                    .file_name()
                    .map(|value| value.replace(['/', '\\'], "_"))
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
                    .map_err(|error| ApiError::internal(error.to_string()))?
                {
                    batch_bytes += chunk.len() as u64;
                    if batch_bytes > state.upload_batch_limit_bytes {
                        return Err(crate::application::ModelImportError::Invalid(
                            "上传批次超过限制".into(),
                        )
                        .into());
                    }
                    writer
                        .write_chunk(&chunk)
                        .map_err(crate::application::ArtifactError::from)?;
                }
                files.push(UploadFile {
                    client_file_id: client_file_id.to_string(),
                    file_name,
                    relative_path: relative_paths.remove(client_file_id),
                    media_type,
                    staged: writer
                        .finish()
                        .map_err(crate::application::ArtifactError::from)?,
                });
            }
        }
    }
    for file in &mut files {
        if let Some(path) = relative_paths.remove(&file.client_file_id) {
            let candidate = std::path::Path::new(&path);
            if candidate.is_absolute()
                || candidate
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(crate::application::ModelImportError::Invalid(
                    "relativePaths 必须是规范相对路径".into(),
                )
                .into());
            }
            file.relative_path = Some(path);
        }
    }
    if !relative_paths.is_empty() {
        return Err(crate::application::ModelImportError::Invalid(
            "relativePaths 包含未知 clientFileId".into(),
        )
        .into());
    }
    if expected_draft_id.is_some() != expected_draft_revision.is_some() {
        return Err(crate::application::ModelImportError::Invalid(
            "expectedDraftId 与 expectedDraftRevision 必须同时提供".into(),
        )
        .into());
    }
    let expected = expected_draft_id
        .zip(expected_draft_revision)
        .map(|(id, revision)| DraftRef { id, revision });
    let service = state.model_import_service.clone();
    let view = service
        .accept(
            &user,
            session_id,
            client_import_id.ok_or_else(|| {
                crate::application::ModelImportError::Invalid("缺少 clientImportId".into())
            })?,
            expected,
            UploadBatch {
                client_batch_id: "v3-model-import".into(),
                files,
            },
        )
        .await?;
    let import_id = view
        .id
        .parse::<i32>()
        .map_err(|_| ApiError::internal("import ID"))?;
    let process_user = user.clone();
    tokio::spawn(async move {
        let _ = service.process(&process_user, import_id).await;
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"ok":true,"modelImport":view})),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "modelImport": state.model_import_service.get(&user, id).await?
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveBody {
    expected_draft_id: Option<i32>,
    expected_draft_revision: Option<i64>,
    #[serde(default)]
    candidate_ids: Vec<String>,
}

pub async fn resolve(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<ResolveBody>,
) -> Result<Json<Value>, ApiError> {
    if body.expected_draft_id.is_some() != body.expected_draft_revision.is_some() {
        return Err(crate::application::ModelImportError::Invalid(
            "expectedDraftId 与 expectedDraftRevision 必须同时提供".into(),
        )
        .into());
    }
    let expected = body
        .expected_draft_id
        .zip(body.expected_draft_revision)
        .map(|(id, revision)| DraftRef { id, revision });
    if body.candidate_ids.is_empty() {
        let current = state.model_import_service.get(&user, id).await?;
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: "模型导入仍需选择表".into(),
            code: "model_import_resolution_required",
            details: Some(json!({"modelImport":current})),
        });
    }
    let draft = state
        .model_import_service
        .resolve_and_apply(&user, id, expected, body.candidate_ids)
        .await?;
    Ok(Json(json!({
        "modelImport": state.model_import_service.get(&user, id).await?,
        "draft": draft
    })))
}

pub async fn discard(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.model_import_service.discard(&user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
