//! 会话与消息 handler：CRUD + 触发一次对话（后台 agent run，经 SSE 推送）+ 附件上传。

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::application::{AppEvent, AppState, AttachmentFile, EventWriter, ms_to_iso, now_ms};
use crate::db::models::{Message, NewMessage};

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

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let sessions = state.session_repo.list().await?;
    let statuses = state.task_repo.latest_statuses().await?;
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
    Json(body): Json<CreateSessionBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();
    let session = state.session_repo.create(&body.title, now).await?;
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
    Path(session_id): Path<i32>,
    Json(body): Json<RenameSessionBody>,
) -> Result<Json<Value>, ApiError> {
    let session = state
        .session_repo
        .rename(session_id, &body.title, now_ms())
        .await?;
    let latest = state
        .task_repo
        .latest_status_for_session(session_id)
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
    Path(session_id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.session_repo.delete(session_id).await?;
    state.events.unregister(session_id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- 附件上传 ----

/// 人类可读的文件大小。
fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// 由 MIME 类型推导前端 `AttachmentFile.file_type`（无则按扩展名）。
fn file_type(content_type: Option<&str>, file_name: &str) -> String {
    if let Some(ct) = content_type.filter(|ct| !ct.is_empty()) {
        return ct.to_string();
    }
    let ext = file_name.rsplit('.').next().unwrap_or("");
    if ext.is_empty() {
        "file".to_string()
    } else {
        ext.to_string()
    }
}

/// `POST /api/sessions/{id}/attachments`（multipart 字段 `file`）→ 201 attachment 事件。
pub async fn upload_attachment(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // 会话必须存在，否则落盘到孤儿目录。
    state.session_repo.find(session_id).await?.ok_or_else(|| {
        ApiError::not_found("会话不存在")
    })?;

    // 取第一个 `file` 字段；在循环内转成 owned 值，避免字段借用逃逸到循环外。
    let mut extracted: Option<(String, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("读取上传字段失败: {e}")))?
    {
        if field.name() == Some("file") {
            let file_name = field
                .file_name()
                .map(|n| n.replace(['/', '\\'], "_"))
                .unwrap_or_else(|| "upload".to_string());
            let content_type = field.content_type().map(|c| c.to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::internal(format!("读取上传内容失败: {e}")))?
                .to_vec();
            extracted = Some((file_name, content_type, bytes));
            break;
        }
    }
    let (file_name, content_type, bytes) =
        extracted.ok_or_else(|| ApiError::not_found("缺少 file 字段"))?;

    let size = human_size(bytes.len());
    let ftype = file_type(content_type.as_deref(), &file_name);

    // 写 `data_dir/uploads/{session_id}/{unix_nanos}_{name}`，避免重名覆盖与路径穿越。
    let uploads_dir = state.data_dir.join("uploads").join(session_id.to_string());
    let disk_path = uploads_dir.join(format!("{}_{}", now_ms(), file_name));
    let bytes_for_write = bytes.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        std::fs::create_dir_all(&uploads_dir)?;
        std::fs::write(&disk_path, bytes_for_write)
    })
    .await
    .map_err(|e| ApiError::internal(format!("写盘任务失败: {e}")))?
    .map_err(|e| ApiError::internal(format!("附件写盘失败: {e}")))?;

    // 落库 + 发布 attachment 事件。
    let file = AttachmentFile {
        file_name,
        file_size: size,
        file_type: ftype,
    };
    let writer = EventWriter::new(session_id, state.message_repo.clone(), state.events.clone());
    let event = writer.attachment(file).await?;

    Ok((StatusCode::CREATED, Json(json!(event))))
}

// ---- 消息 ----

pub async fn messages(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    let msgs = state.message_repo.list_by_session(session_id).await?;
    let items: Vec<Value> = msgs.iter().map(json_from_message).collect();
    Ok(Json(json!({ "messages": items })))
}

#[derive(Deserialize)]
pub struct PostMessageBody {
    message: String,
}

pub async fn post_message(
    State(state): State<AppState>,
    Path(session_id): Path<i32>,
    Json(body): Json<PostMessageBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();

    // 1. 落用户消息。
    let user_msg = state
        .message_repo
        .create(NewMessage {
            session_id,
            task_id: None,
            role: "user".to_string(),
            kind: "text".to_string(),
            content: body.message.clone(),
            payload_json: None,
            created_at_ms: now,
        })
        .await?;

    // 2. 发布 user 事件（前端已本地显示，但事件流保持一致）。
    state
        .events
        .publish(session_id, AppEvent::from_message(&user_msg));

    // 3. 更新会话活跃时间。
    state.session_repo.touch(session_id, now).await?;

    // 4. 后台 agent run：真正的流式驱动（第 3 步）。
    let chat = state.chat();
    let prompt_id = user_msg.id;
    let prompt = body.message;
    tokio::spawn(async move {
        chat.run(session_id, prompt_id, prompt).await;
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
