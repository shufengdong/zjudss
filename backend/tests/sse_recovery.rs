mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use futures::StreamExt;
use tower::ServiceExt;
use zjudss::application::{EventWriter, now_ms};
use zjudss::auth::UserId;
use zjudss::db::models::NewMessage;
use zjudss::db::repositories::RepoError;

use common::TestApp;

fn instance_scope() -> UserId {
    UserId("instance".to_string())
}

async fn read_events(body: Body, count: usize) -> String {
    let mut stream = body.into_data_stream();
    let mut output = String::new();
    while output.matches("\n\n").count() < count {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("SSE event timeout")
            .expect("SSE stream ended")
            .expect("read SSE body");
        output.push_str(std::str::from_utf8(&next).expect("SSE is UTF-8"));
    }
    output
}

#[tokio::test]
async fn reconnect_replays_then_switches_to_live_without_duplicates() {
    let app = TestApp::spawn().await;
    let scope = instance_scope();
    let session = app
        .state
        .session_repo
        .create(&scope, "SSE", now_ms())
        .await
        .expect("create session");
    let writer = EventWriter::new(
        scope.clone(),
        session.id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );
    let first = writer.assistant("one").await.expect("first message");
    let second = writer.assistant("two").await.expect("second message");
    let cookie = app.login_cookie("instance").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/events", session.id))
                .header("last-event-id", first.message_id().unwrap().to_string())
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("SSE request"),
        )
        .await
        .expect("SSE response");

    // handler 已先订阅并完成补发查询；此消息必须从实时通道接续。
    let third = writer.assistant("three").await.expect("live message");
    let output = read_events(response.into_body(), 2).await;
    let second_id = second.message_id().unwrap();
    let third_id = third.message_id().unwrap();
    assert_eq!(output.matches(&format!("id: {second_id}")).count(), 1);
    assert_eq!(output.matches(&format!("id: {third_id}")).count(), 1);
    assert!(output.find("two").unwrap() < output.find("three").unwrap());
}

#[tokio::test]
async fn foreign_or_missing_cursor_requests_resync_without_leaking_messages() {
    let app = TestApp::spawn().await;
    let scope = instance_scope();
    let first_session = app
        .state
        .session_repo
        .create(&scope, "one", now_ms())
        .await
        .expect("first session");
    let second_session = app
        .state
        .session_repo
        .create(&scope, "two", now_ms())
        .await
        .expect("second session");
    let foreign = app
        .state
        .message_repo
        .create(
            &scope,
            NewMessage {
                session_id: second_session.id,
                run_id: None,
                role: "assistant".to_string(),
                kind: "text".to_string(),
                content: "foreign-secret".to_string(),
                payload_json: None,
                created_at_ms: now_ms(),
            },
        )
        .await
        .expect("foreign message");

    let other_scope = UserId("bob".to_string());
    let cookie = app.login_cookie("instance").await;
    assert!(matches!(
        app.state
            .message_repo
            .list_after(&other_scope, first_session.id, foreign.id)
            .await,
        Err(RepoError::NotFound)
    ));

    for cursor in [foreign.id.to_string(), "2147483647".to_string()] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/sessions/{}/events", first_session.id))
                    .header("last-event-id", cursor)
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .expect("SSE request"),
            )
            .await
            .expect("SSE response");
        let output = read_events(response.into_body(), 1).await;
        assert!(output.contains("event: resync-required"));
        assert!(output.contains("cursor_unavailable"));
        assert!(!output.contains("foreign-secret"));
    }
}
