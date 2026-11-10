//! 会话、历史消息与对话启动的 HTTP 边界。
//!
//! 所有路由均要求当前用户，并通过 session/history repository 或 application service 保持用户
//! 作用域；跨用户与不存在资源统一表现为 404。本模块负责分页 DTO、删除影响 token 和 Agent
//! admission 映射，不拥有消息持久化规则、删除事务或 Agent/引擎执行逻辑；实时投递由相邻的
//! SSE 模块负责。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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

/// `GET /api/sessions`：列出当前用户的会话摘要。
///
/// 成功返回 200，并把每个会话的最近运行状态投影为前端三态；只读取当前用户数据，不创建或
/// 激活任何资源。
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

/// `POST /api/sessions`：为当前用户创建会话。
///
/// 标题缺省为空字符串；成功持久化会话并返回 201，新会话状态为 `completed`。
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

/// `PATCH /api/sessions/{id}`：修改当前用户会话标题。
///
/// 成功返回 200 和更新后的会话摘要，状态仍由最近运行推导；不存在或跨用户会话统一返回
/// 404。
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDeletionPreview {
    impact_token: String,
    model_count: i64,
    data_resource_count: i64,
    script_count: i64,
    run_count: i64,
    result_count: i64,
    analysis_artifact_count: i64,
    logical_bytes: i64,
    expires_at: String,
}

/// `POST /api/sessions/{id}/deletion-preview`：生成会话删除影响预览。
///
/// 当前用户拥有会话时返回 200、影响计数、逻辑字节数和五分钟有效的 opaque token；仓储会
/// 保存 token hash 与影响快照，后续删除必须使用同一预览。不存在或跨用户会话统一返回 404。
pub async fn deletion_preview(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(session_id): Path<i32>,
) -> Result<Json<SessionDeletionPreview>, ApiError> {
    let token = random_token();
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    let now = now_ms();
    let expires_at = now + 5 * 60 * 1000;
    let impact = state
        .session_repo
        .preview_deletion(&principal.id, session_id, token_hash, expires_at, now)
        .await?;
    Ok(Json(SessionDeletionPreview {
        impact_token: token,
        model_count: impact.model_count,
        data_resource_count: impact.data_resource_count,
        script_count: impact.script_count,
        run_count: impact.run_count,
        result_count: impact.result_count,
        analysis_artifact_count: impact.analysis_artifact_count,
        logical_bytes: impact.logical_bytes,
        expires_at: ms_to_iso(expires_at),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionQuery {
    impact_token: String,
}

/// `DELETE /api/sessions/{id}?impactToken=...`：确认删除当前用户会话。
///
/// token 必须来自尚未过期且影响未变化的预览。成功或同一删除的幂等重放返回 204；首次确认
/// 会取消该会话的 Agent/引擎工作、注销 SSE 通道、执行仓储清理并释放 artifact 引用。无效
/// token 或影响变化返回对应稳定冲突码，跨用户与不存在会话统一返回 404。
pub async fn delete(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(session_id): Path<i32>,
    Query(query): Query<DeleteSessionQuery>,
) -> Result<StatusCode, ApiError> {
    if query.impact_token.len() != 64 {
        return Err(ApiError::public(
            StatusCode::CONFLICT,
            "invalid_deletion_impact_token",
            "删除影响令牌无效",
        ));
    }
    let token_hash = hex::encode(Sha256::digest(query.impact_token.as_bytes()));
    let should_delete = state
        .session_repo
        .begin_delete(&principal.id, session_id, token_hash, now_ms())
        .await?;
    if !should_delete {
        return Ok(StatusCode::NO_CONTENT);
    }
    state
        .session_cancellations
        .cancel(&principal.id, session_id);
    state.events.unregister(session_id);
    let artifacts = state.session_repo.delete(&principal.id, session_id).await?;
    for artifact_id in artifacts {
        let _ = state
            .artifact_service
            .release(principal.storage_scope_id, artifact_id)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source unavailable");
    hex::encode(bytes)
}

// ---- 消息 ----

/// `GET /api/sessions/{id}/messages`：读取当前用户会话的连续消息窗口。
///
/// `beforeId` 向前分页，`aroundId` 必须精确属于该会话，`limit` 由 history service 限制在
/// 1..=100；成功返回 200、消息和下一页锚点，只读且不触发 Agent。非法锚点或跨用户资源不会
/// 泄露其他会话内容。
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
/// `GET /api/history/search`：搜索当前用户可读的会话历史。
///
/// 接受正文查询及可选脚本名、状态、时间范围和数量限制；成功返回 200 和有界摘要，不返回
/// 脚本全文或原始 stderr。非法查询返回稳定的 400 错误，索引不可用返回 503
/// `history_search_unavailable`。
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

/// `POST /api/sessions/{id}/messages`：追加用户消息并启动一次后台 Agent 对话。
///
/// admission 在写消息前完成；队列已满返回 429 `agent_queue_full` 且不持久化用户消息。成功
/// 先落库并广播用户消息、更新会话活跃时间，再异步运行 Agent，返回 202 和当前会话标题；后续
/// durable 卡片与回复通过消息历史和 SSE 可恢复。跨用户与不存在会话统一返回 404。
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
