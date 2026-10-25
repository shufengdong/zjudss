use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::application::{AppState, CreateScriptRequest, ScriptSource};
use crate::auth::CurrentUser;

use super::ApiError;

#[derive(serde::Deserialize)]
pub struct CreateScriptTextBody {
    name: String,
    content: String,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"scripts":state.script_service.list(&user,session_id).await?}),
    ))
}

pub async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut name = None;
    let mut content = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
    {
        if field.name() == Some("file") {
            let file_name = field
                .file_name()
                .unwrap_or("script.rss")
                .replace(['/', '\\'], "_");
            let bytes = field
                .bytes()
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            if content.is_some() {
                return Err(crate::application::ScriptError::Invalid(
                    "一次只能上传一个 .rss 脚本".into(),
                )
                .into());
            }
            name = Some(file_name);
            content = Some(String::from_utf8(bytes.to_vec()).map_err(|_| {
                crate::application::ScriptError::Invalid("脚本必须是 UTF-8 文本".into())
            })?);
        }
    }
    let summary = state
        .script_service
        .create(
            &user,
            session_id,
            CreateScriptRequest {
                name: name.ok_or_else(|| {
                    crate::application::ScriptError::Invalid("缺少 .rss 文件".into())
                })?,
                content: content.ok_or_else(|| {
                    crate::application::ScriptError::Invalid("缺少 .rss 文件".into())
                })?,
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"script":summary}))))
}

pub async fn create_text(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Json(body): Json<CreateScriptTextBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let summary = state
        .script_service
        .create(
            &user,
            session_id,
            CreateScriptRequest {
                name: body.name,
                content: body.content,
                source: ScriptSource::Manual,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"script": summary}))))
}

pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"script":state.script_service.detail(&user,id).await?}),
    ))
}
