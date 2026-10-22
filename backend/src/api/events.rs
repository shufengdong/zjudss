//! SSE 事件流端点：把 EventBus 的 broadcast 流转成 Server-Sent Events。

use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::application::AppState;
use crate::auth::CurrentPrincipal;

use super::ApiError;

pub async fn stream(
    State(state): State<AppState>,
    CurrentPrincipal(principal): CurrentPrincipal,
    Path(session_id): Path<i32>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    state
        .session_repo
        .find(&principal.access, session_id)
        .await?
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    let rx = state.events.subscribe(session_id);
    let stream = BroadcastStream::new(rx).filter_map(|item| {
        let event = item.ok()?; // 忽略 lagged / 通道关闭
        let data = serde_json::to_string(&event).ok()?;
        Some(Ok(SseEvent::default().event("message").data(data)))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
