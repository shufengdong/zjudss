//! 会话与消息 handler：CRUD + 触发一次对话（后台 agent run，经 SSE 推送）。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{AppEvent, AppState, EventWriter, ms_to_iso, now_ms};
use crate::auth::CurrentUser;
use crate::db::models::{Message, NewMessage};

use super::ApiError;

// ---- 会话 ----

/// 最近一次运行状态 → 会话状态（前端 `SessionStatus`）。
fn derive_status(latest: Option<&str>) -> &'static str {
    match latest {
        Some("succeeded") => "completed",
        Some("pending") | Some("queued") | Some("running") => "running",
        Some("failed") | Some("cancelled") => "failed",
        _ => "completed",
    }
}

pub async fn list(
    State(state): State<AppState>,
    principal: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let sessions = state.session_repo.list(&principal.id).await?;
    let statuses = state.run_service.latest_statuses(&principal).await?;
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
    principal: CurrentUser,
    Json(body): Json<CreateSessionBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();
    let session = state
        .session_repo
        .create(&principal.id, &body.title, now)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": session.id.to_string(),
            "title": session.title,
            // 新会话尚无运行，直接 completed。
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
    principal: CurrentUser,
    Path(session_id): Path<i32>,
    Json(body): Json<RenameSessionBody>,
) -> Result<Json<Value>, ApiError> {
    let session = state
        .session_repo
        .rename(&principal.id, session_id, &body.title, now_ms())
        .await?;
    let statuses = state.run_service.latest_statuses(&principal).await?;
    let status = derive_status(statuses.get(&session_id).map(String::as_str));
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
    principal: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    let artifacts = state.session_repo.delete(&principal.id, session_id).await?;
    for artifact_id in artifacts {
        let _ = state
            .artifact_service
            .release(principal.storage_scope_id, artifact_id)
            .await;
    }
    state.events.unregister(session_id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- 消息 ----

pub async fn messages(
    State(state): State<AppState>,
    principal: CurrentUser,
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
    script_name: Option<String>,
    status: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    limit: Option<i64>,
}
pub async fn search_history(
    State(state): State<AppState>,
    principal: CurrentUser,
    Query(q): Query<HistorySearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let hits = state
        .history_service
        .search_sessions(
            &principal,
            q.query,
            crate::db::repositories::SearchFilters {
                script_name: q.script_name,
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
    principal: CurrentUser,
    Path(session_id): Path<i32>,
    Json(body): Json<PostMessageBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();

    // 先校验归属并完成 admission；队列满时不能写入用户消息。
    state
        .session_repo
        .find(&principal.id, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    let ticket = state.scheduler.try_admit(&principal.id, session_id)?;
    let queued = ticket.is_queued();

    // 1. admission 成功后落用户消息。
    let user_msg = state
        .message_repo
        .create(
            &principal.id,
            NewMessage {
                session_id,
                run_id: None,
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
            principal.id.clone(),
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
        .touch(&principal.id, session_id, now)
        .await?;
    let session_title = state
        .session_repo
        .find(&principal.id, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?
        .title;

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

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "ok": true, "sessionTitle": session_title })),
    ))
}

/// 把落库消息重组为前端消息形状（覆盖全部 6 种类型，经 `AppEvent::from_message`）。
fn json_from_message(m: &Message) -> Value {
    serde_json::to_value(AppEvent::from_message(m)).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::derive_status;

    /// 运行状态 → 会话状态映射，锁定前端 `SessionStatus` 三态。
    #[test]
    fn derive_status_maps_run_status() {
        assert_eq!(derive_status(Some("succeeded")), "completed");
        assert_eq!(derive_status(Some("pending")), "running");
        assert_eq!(derive_status(Some("running")), "running");
        assert_eq!(derive_status(Some("failed")), "failed");
        assert_eq!(derive_status(Some("cancelled")), "failed");
        assert_eq!(derive_status(None), "completed");
        assert_eq!(derive_status(Some("unknown")), "completed");
    }
}
