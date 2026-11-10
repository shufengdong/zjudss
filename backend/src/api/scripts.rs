//! 不可变脚本资源与失败受管脚本重试预览的 HTTP 边界。
//!
//! 所有路由要求当前用户。手工/上传入口只校验名称、UTF-8 和容量后发布不可变 `.rss`，不会
//! 创建即执行或在 API 层做静态 RSS 语法判断；运行和受管重试由独立 application service
//! 拥有。

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

/// `GET /api/sessions/{id}/scripts`：列出当前用户会话的脚本摘要。
///
/// 成功返回 200 和不可变脚本的有界预览；不读取全文、不创建运行，跨用户与不存在会话统一
/// 返回 404。
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"scripts":state.script_service.list(&user,session_id).await?}),
    ))
}

/// `POST /api/sessions/{id}/scripts`：上传单个 UTF-8 `.rss` 脚本。
///
/// 路由关闭 Axum 默认 body limit，但 script service 仍执行容量与名称限制；multipart 只接受
/// 一个 `file`。成功不可变发布 `sourceKind=upload` 脚本并返回 201，不运行或激活模型。缺失、
/// 多文件、非 UTF-8 或非法脚本名返回 422 `invalid_script`，跨用户会话统一返回 404。
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
                study_plan: None,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"script":summary}))))
}

/// `POST /api/sessions/{id}/scripts/text`：从 JSON 文本创建手工脚本。
///
/// 请求包含名称和 UTF-8 正文；成功不可变发布 `sourceKind=manual` 脚本并返回 201，不执行
/// tensoreval 或创建 run。非法名称/大小返回 422 `invalid_script`，跨用户与不存在会话统一
/// 返回 404。
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
                study_plan: None,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(json!({"script": summary}))))
}

/// `GET /api/scripts/{id}`：读取当前用户不可变脚本详情与全文。
///
/// 成功返回 200；该读取不创建运行或修改脚本，跨用户与不存在脚本统一返回 404。
pub async fn detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"script":state.script_service.detail(&user,id).await?}),
    ))
}

/// `POST /api/scripts/{id}/retry-previews`：为失败的受管 Agent 脚本准备重新尝试。
///
/// 请求携带幂等 `clientRetryId`。当前输入未变化时可直接创建派生 workflow；有变化时返回 200、
/// 持久差异和短期 preview ID，等待用户确认。重放返回原 workflow/preview，不重复消耗修正
/// 预算；非受管或不可重试脚本返回受控 422，跨用户与不存在脚本统一返回 404。
pub async fn retry_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<crate::application::RetryStudyRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(
            state
                .managed_workflow_service
                .preview_retry(&user, id, body)
                .await?,
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
    ))
}
