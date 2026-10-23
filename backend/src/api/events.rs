//! 可恢复 SSE：先订阅实时通道，再按 Last-Event-ID 补发持久消息，最后去重接续实时事件。

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, Sse};
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
    let revocations = state.auth.subscribe_revocations();
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
    let auth = state.auth.clone();
    let auth_session_hash = principal.auth_session_hash;
    let heartbeat_interval = Duration::from_millis(250);
    let heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    let live = futures::stream::unfold(
        (
            BroadcastStream::new(rx),
            revocations,
            heartbeat,
            auth,
            auth_session_hash,
            live_seen,
        ),
        |(mut events, mut revocations, mut heartbeat, auth, auth_session_hash, live_seen)| async move {
            loop {
                tokio::select! {
                    item = events.next() => match item {
                        Some(Ok(event)) => {
                            let Some(id) = event.message_id() else { continue };
                            if id <= live_seen.fetch_max(id, Ordering::SeqCst) {
                                continue;
                            }
                            let Some(frame) = message_frame(event) else { continue };
                            return Some((
                                Ok(frame),
                                (events, revocations, heartbeat, auth, auth_session_hash, live_seen),
                            ));
                        }
                        Some(Err(BroadcastStreamRecvError::Lagged(_))) => {
                            return Some((
                                Ok(resync_frame()),
                                (events, revocations, heartbeat, auth, auth_session_hash, live_seen),
                            ));
                        }
                        None => return None,
                    },
                    revoked = revocations.recv() => match revoked {
                        Ok(hash) if hash == auth_session_hash => return None,
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => continue,
                    },
                    _ = heartbeat.tick() => {
                        match auth.is_session_active(&auth_session_hash).await {
                            Ok(true) => {
                                let frame = SseEvent::default().comment("keep-alive");
                                return Some((
                                    Ok(frame),
                                    (events, revocations, heartbeat, auth, auth_session_hash, live_seen),
                                ));
                            }
                            Ok(false) | Err(_) => return None,
                        }
                    }
                }
            }
        },
    );
    let stream: EventStream = Box::pin(futures::stream::iter(initial).chain(live));
    Ok(Sse::new(stream))
}
