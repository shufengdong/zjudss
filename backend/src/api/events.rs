//! 可恢复 SSE：先订阅实时通道，再按 Last-Event-ID 补发持久消息，最后去重接续实时事件。

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::application::{AppEvent, AppState};
use crate::auth::CurrentUser;
use crate::db::repositories::RepoError;

use super::ApiError;

type EventStream = Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>>;

fn message_frame(event: AppEvent) -> Option<SseEvent> {
    let id = event.message_id()?;
    let data = serde_json::to_string(&event).ok()?;
    Some(
        SseEvent::default()
            .event("message")
            .id(id.to_string())
            .data(data),
    )
}

fn resync_frame() -> SseEvent {
    SseEvent::default()
        .event("resync-required")
        .data(r#"{"reason":"cursor_unavailable"}"#)
}

pub async fn stream(
    State(state): State<AppState>,
    principal: CurrentUser,
    Path(session_id): Path<i32>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    state
        .session_repo
        .find(&principal.id, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;

    // 必须先订阅；补发查询期间提交的消息会保留在 rx 中，之后按消息 ID 去重。
    let rx = state.events.subscribe(session_id);
    let raw_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok());
    let parsed_cursor = raw_cursor.and_then(|value| value.parse::<i32>().ok());
    let invalid_cursor = raw_cursor.is_some() && parsed_cursor.is_none_or(|id| id <= 0);

    let mut initial = Vec::new();
    let mut last_id = 0;
    if invalid_cursor {
        initial.push(Ok(resync_frame()));
    } else if let Some(after_id) = parsed_cursor {
        match state
            .message_repo
            .list_after(&principal.id, session_id, after_id)
            .await
        {
            Ok(messages) => {
                last_id = after_id;
                for message in messages {
                    let event = AppEvent::from_message(&message);
                    last_id = last_id.max(message.id);
                    if let Some(frame) = message_frame(event) {
                        initial.push(Ok(frame));
                    }
                }
            }
            Err(RepoError::NotFound) => initial.push(Ok(resync_frame())),
            Err(error) => return Err(error.into()),
        }
    }

    let last_seen = Arc::new(AtomicI32::new(last_id));
    let live_seen = last_seen.clone();
    let live = BroadcastStream::new(rx).filter_map(move |item| {
        let live_seen = live_seen.clone();
        async move {
            match item {
                Ok(event) => {
                    let id = event.message_id()?;
                    if id <= live_seen.fetch_max(id, Ordering::SeqCst) {
                        return None;
                    }
                    message_frame(event).map(Ok)
                }
                Err(BroadcastStreamRecvError::Lagged(_)) => Some(Ok(resync_frame())),
            }
        }
    });
    let stream: EventStream = Box::pin(futures::stream::iter(initial).chain(live));
    // 代理可能等到首个 body chunk 才向浏览器提交响应头。较短的注释心跳使空会话也能
    // 及时触发 EventSource.open，前端随后读取 REST 快照，同时不改变业务事件序列。
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_millis(250))))
}
