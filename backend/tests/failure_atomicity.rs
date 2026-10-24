#![cfg(feature = "e2e-test")]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use zjudss::application::{EventWriter, now_ms};
use zjudss::auth::{CurrentUser, UserId};
use zjudss::testing::{
    FaultInjector, FaultPoint, ScriptScenario, ScriptTurn, ScriptedAgentFactory,
};

use common::TestApp;

fn principal() -> CurrentUser {
    CurrentUser {
        id: UserId("instance".into()),
        name: "Instance".into(),
        storage_scope_id: 1,
        auth_session_hash: "test".into(),
    }
}

#[tokio::test]
async fn injected_event_commit_failure_publishes_nothing_and_keeps_history_readable() {
    let app = TestApp::spawn().await;
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.id, "atomic", now_ms())
        .await
        .unwrap();
    let writer = EventWriter::new(
        p.id.clone(),
        session.id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );
    writer.assistant("已发布历史").await.unwrap();
    let mut subscriber = app.state.events.subscribe(session.id);
    FaultInjector::default().fail_next(FaultPoint::EventCommit);
    assert!(writer.assistant("不得发布").await.is_err());
    assert!(subscriber.try_recv().is_err());
    let history = app
        .state
        .message_repo
        .list_by_session(&p.id, session.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "已发布历史");
    writer.assistant("恢复成功").await.unwrap();
    assert_eq!(
        app.state
            .message_repo
            .list_by_session(&p.id, session.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn provider_errors_release_the_permit_without_exposing_internal_details() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "sensitive-provider-error".into(),
        expected_prompt: None,
        turns: vec![ScriptTurn::Error {
            message: "api_key=top-secret upstream=https://internal.example".into(),
        }],
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.id, "redaction", now_ms())
        .await
        .unwrap();
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/messages", session.id))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(r#"{"message":"触发错误"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let messages = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let messages = app
                .state
                .message_repo
                .list_by_session(&p.id, session.id)
                .await
                .unwrap();
            if messages.iter().any(|message| message.role == "assistant") {
                break messages;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("assistant error persisted");
    let rendered = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rendered.contains("top-secret"));
    assert!(!rendered.contains("internal.example"));
    assert!(rendered.contains("agent 运行失败，请稍后重试"));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.state.scheduler.active_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider error releases scheduler permit");
}
