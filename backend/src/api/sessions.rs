//! 会话与消息 handler：CRUD + 触发一次对话（后台 agent run，经 SSE 推送）+ 附件上传。

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{
    AppEvent, AppState, EventWriter, UploadBatch, UploadFile, ms_to_iso, now_ms,
};
use crate::auth::CurrentPrincipal;
use crate::db::models::{Message, NewMessage};
use crate::storage::StageLimits;

use super::ApiError;

// ---- 会话 ----

/// 任务状态（`simulation_tasks.status`）→ 会话状态（前端 `SessionStatus`）。
fn derive_status(latest: Option<&str>) -> &'static str {
    match latest {
        Some("succeeded") => "completed",
        Some("pending") | Some("running") => "running",
        Some("failed") | Some("cancelled") => "failed",
        _ => "completed",
    }
}

pub async fn list(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
) -> Result<Json<Value>, ApiError> {
    let sessions = state.session_repo.list(&principal.access).await?;
    let statuses = state.task_repo.latest_statuses(&principal.access).await?;
    let items: Vec<Value> = sessions
        .into_iter()
        .map(|s| {
            let status = derive_status(statuses.get(&s.id).map(String::as_str));
            json!({
                "id": s.id.to_string(),
                "title": s.title,
                "status": status,
                "updatedAt": ms_to_iso(s.updated_at_ms),
            })
        })
        .collect();
    Ok(Json(json!({ "sessions": items })))
}

#[derive(Deserialize)]
pub struct CreateSessionBody {
    #[serde(default)]
    title: String,
}

pub async fn create(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Json(body): Json<CreateSessionBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();
    let session = state
        .session_repo
        .create(&principal.access, &body.title, now)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": session.id.to_string(),
            "title": session.title,
            // 新会话无任务，直接 completed。
            "status": "completed",
            "updatedAt": ms_to_iso(session.updated_at_ms),
        })),
    ))
}

#[derive(Deserialize)]
pub struct RenameSessionBody {
    title: String,
}

/// `PATCH /api/sessions/{id}` → 200 改名后的 Session。
pub async fn rename(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
    Json(body): Json<RenameSessionBody>,
) -> Result<Json<Value>, ApiError> {
    let session = state
        .session_repo
        .rename(&principal.access, session_id, &body.title, now_ms())
        .await?;
    let latest = state
        .task_repo
        .latest_status_for_session(&principal.access, session_id)
        .await?;
    let status = derive_status(latest.as_deref());
    Ok(Json(json!({
        "id": session.id.to_string(),
        "title": session.title,
        "status": status,
        "updatedAt": ms_to_iso(session.updated_at_ms),
    })))
}

/// `DELETE /api/sessions/{id}` → 204；未知会话 → 404。同时注销事件通道。
pub async fn delete(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state
        .session_repo
        .delete(&principal.access, session_id)
        .await?;
    state.events.unregister(session_id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- 附件上传 ----

/// `POST /api/sessions/{id}/attachments`：v2 多文件流式上传。
pub async fn upload_attachment(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    state
        .session_repo
        .find(&principal.access, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("会话不存在"))?;

    let mut client_batch_id = None;
    let mut relative_paths = std::collections::HashMap::<String, String>::new();
    let mut files = Vec::new();
    let mut batch_bytes = 0_u64;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("读取上传字段失败: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "clientBatchId" {
            client_batch_id = Some(
                field
                    .text()
                    .await
                    .map_err(|e| ApiError::internal(format!("读取 clientBatchId 失败: {e}")))?,
            );
        } else if name == "relativePaths" {
            let text = field
                .text()
                .await
                .map_err(|e| ApiError::internal(format!("读取 relativePaths 失败: {e}")))?;
            relative_paths = serde_json::from_str(&text).map_err(|_| {
                crate::application::IngestionError::InvalidUpload(
                    "relativePaths 必须是 clientFileId 到相对路径的 JSON 对象".into(),
                )
            })?;
        } else if let Some(client_file_id) = name.strip_prefix("file.") {
            if client_file_id.is_empty() {
                return Err(crate::application::IngestionError::InvalidUpload(
                    "file 字段缺少 clientFileId".into(),
                )
                .into());
            }
            let file_name = field
                .file_name()
                .map(|n| n.replace(['/', '\\'], "_"))
                .unwrap_or_else(|| "upload".to_string());
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
                .map_err(|e| ApiError::internal(format!("读取上传内容失败: {e}")))?
            {
                batch_bytes = batch_bytes.checked_add(chunk.len() as u64).ok_or_else(|| {
                    crate::application::IngestionError::InvalidUpload("上传批次大小溢出".into())
                })?;
                if batch_bytes > state.upload_batch_limit_bytes {
                    return Err(crate::application::IngestionError::UploadTooLarge(format!(
                        "上传批次超过 {} 字节限制",
                        state.upload_batch_limit_bytes
                    ))
                    .into());
                }
                writer
                    .write_chunk(&chunk)
                    .map_err(crate::application::ArtifactError::from)?;
            }
            files.push(UploadFile {
                client_file_id: client_file_id.to_string(),
                file_name,
                relative_path: None,
                media_type,
                staged: writer
                    .finish()
                    .map_err(crate::application::ArtifactError::from)?,
            });
        } else if name == "file" {
            return Err(crate::application::IngestionError::InvalidUpload(
                "v2 不接受旧 file 字段，请使用 file.<clientFileId>".into(),
            )
            .into());
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
                return Err(crate::application::IngestionError::InvalidUpload(
                    "relativePaths 必须是规范相对路径".into(),
                )
                .into());
            }
            file.relative_path = Some(path);
        }
    }
    if !relative_paths.is_empty() {
        return Err(crate::application::IngestionError::InvalidUpload(
            "relativePaths 包含未知 clientFileId".into(),
        )
        .into());
    }
    let accepted = state
        .ingestion_service
        .accept_batch(
            &principal,
            session_id,
            UploadBatch {
                client_batch_id: client_batch_id.ok_or_else(|| {
                    crate::application::IngestionError::InvalidUpload("缺少 clientBatchId".into())
                })?,
                files,
            },
        )
        .await?;
    let ingestion_id: i32 = accepted
        .id
        .parse()
        .map_err(|_| ApiError::internal("ingestion ID 无效"))?;
    let service = state.ingestion_service.clone();
    tokio::spawn(async move {
        service.process(principal, ingestion_id).await;
    });
    let accepted_items: Vec<_> = accepted
        .items
        .iter()
        .filter(|item| item.role == "uploaded")
        .map(|item| {
            json!({
                "clientFileId": item.client_file_id,
                "artifactId": item.artifact_id,
                "fileName": item.file_name,
                "deduplicated": false,
            })
        })
        .collect();
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "ok": true,
            "ingestionId": accepted.id,
            "accepted": accepted_items,
        })),
    ))
}

// ---- 消息 ----

pub async fn messages(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
    Query(query): Query<MessagePageQuery>,
) -> Result<Json<Value>, ApiError> {
    let msgs = state
        .history_service
        .list_messages(
            &principal,
            session_id,
            query.before_id,
            query.around_id,
            query.limit.unwrap_or(50),
        )
        .await?;
    let items: Vec<Value> = msgs.iter().map(json_from_message).collect();
    let next = msgs.first().map(|m| m.id.to_string());
    Ok(Json(
        json!({ "messages": items,"nextBeforeId":next,"hasMoreBefore":msgs.len()>=query.limit.unwrap_or(50).clamp(1,100) as usize }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePageQuery {
    before_id: Option<i32>,
    around_id: Option<i32>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySearchQuery {
    query: String,
    study_type: Option<String>,
    status: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    limit: Option<i64>,
}
pub async fn search_history(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Query(q): Query<HistorySearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let hits = state
        .history_service
        .search_sessions(
            &principal,
            q.query,
            crate::db::repositories::SearchFilters {
                study_type: q.study_type,
                status: q.status,
                from_ms: q.from_ms,
                to_ms: q.to_ms,
            },
            q.limit.unwrap_or(20),
        )
        .await?;
    Ok(Json(json!({"items":hits})))
}

#[derive(Deserialize)]
pub struct PostMessageBody {
    message: String,
}

pub async fn post_message(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
    Json(body): Json<PostMessageBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();

    // 先校验归属并完成 admission；队列满时不能写入用户消息。
    state
        .session_repo
        .find(&principal.access, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    let ticket = state.scheduler.try_admit(&principal.access, session_id)?;
    let queued = ticket.is_queued();

    // 1. admission 成功后落用户消息。
    let user_msg = state
        .message_repo
        .create(
            &principal.access,
            NewMessage {
                session_id,
                task_id: None,
                role: "user".to_string(),
                kind: "text".to_string(),
                content: body.message.clone(),
                payload_json: None,
                created_at_ms: now,
            },
        )
        .await?;

    // 2. 发布 user 事件（前端已本地显示，但事件流保持一致）。
    state
        .events
        .publish(session_id, AppEvent::from_message(&user_msg));

    let queue_call_id = format!("agent-admission:{}", user_msg.id);
    let queue_writer = if queued {
        let writer = EventWriter::new(
            principal.access.clone(),
            session_id,
            state.message_repo.clone(),
            state.events.clone(),
        );
        writer
            .tool_call(
                &queue_call_id,
                "agent_admission",
                json!({}),
                crate::application::ToolStatus::Running,
                Some("Agent 请求已进入等待队列".to_string()),
            )
            .await?;
        Some(writer)
    } else {
        None
    };

    // 3. 更新会话活跃时间。
    state
        .session_repo
        .touch(&principal.access, session_id, now)
        .await?;

    // 4. 后台 agent run：真正的流式驱动（第 3 步）。
    let chat = state.chat(principal);
    let prompt_id = user_msg.id;
    let prompt = body.message;
    tokio::spawn(async move {
        ticket
            .run(async move {
                if let Some(writer) = queue_writer {
                    let _ = writer
                        .tool_call(
                            &queue_call_id,
                            "agent_admission",
                            json!({}),
                            crate::application::ToolStatus::Success,
                            Some("Agent 已开始处理".to_string()),
                        )
                        .await;
                }
                chat.run(session_id, prompt_id, prompt).await;
            })
            .await;
    });

    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

/// 把落库消息重组为前端消息形状（覆盖全部 6 种类型，经 `AppEvent::from_message`）。
fn json_from_message(m: &Message) -> Value {
    serde_json::to_value(AppEvent::from_message(m)).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::derive_status;

    /// 任务状态 → 会话状态映射，锁定前端 `SessionStatus` 三态。
    #[test]
    fn derive_status_maps_task_status() {
        assert_eq!(derive_status(Some("succeeded")), "completed");
        assert_eq!(derive_status(Some("pending")), "running");
        assert_eq!(derive_status(Some("running")), "running");
        assert_eq!(derive_status(Some("failed")), "failed");
        assert_eq!(derive_status(Some("cancelled")), "failed");
        assert_eq!(derive_status(None), "completed");
        assert_eq!(derive_status(Some("unknown")), "completed");
    }
}
