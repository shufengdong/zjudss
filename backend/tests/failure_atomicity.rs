#![cfg(feature = "e2e-test")]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use tower::ServiceExt;
use zjudss::application::{EventWriter, now_ms};
use zjudss::auth::{CurrentUser, UserId};
use zjudss::db::repositories::RunRepo;
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
async fn cancel_and_result_publication_preserve_the_first_terminal_state() {
    let app = TestApp::spawn().await;
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.id, "run race", 1)
        .await
        .unwrap();
    let (cancel_first, complete_first) = {
        let mut conn = app.pool.get().unwrap();
        for client_id in ["cancel-first", "complete-first"] {
            diesel::sql_query(
                "INSERT INTO simulation_runs(session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,run_kind,input_snapshot_json,created_at_ms) VALUES(?1,?2,'running','run','test','[]','managed','{}',1)",
            )
            .bind::<diesel::sql_types::Integer, _>(session.id)
            .bind::<diesel::sql_types::Text, _>(client_id)
            .execute(&mut conn)
            .unwrap();
        }
        let ids = zjudss::db::schema::simulation_runs::table
            .filter(zjudss::db::schema::simulation_runs::session_id.eq(session.id))
            .order(zjudss::db::schema::simulation_runs::id.asc())
            .select(zjudss::db::schema::simulation_runs::id)
            .load::<i32>(&mut conn)
            .unwrap();
        (ids[0], ids[1])
    };
    let repo = RunRepo::new(app.pool.clone());

    let cancelled = repo.cancel(&p.id, cancel_first, 10).await.unwrap();
    assert_eq!(cancelled.status, "failed");
    assert_eq!(cancelled.stage, "cancelled_by_user");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(cancelled.error_json.as_deref().unwrap())
            .unwrap()["code"],
        "cancelled_by_user"
    );
    assert!(
        repo.complete(&p.id, cancel_first, vec![], 11)
            .await
            .is_err()
    );
    assert_eq!(
        repo.find(&p.id, cancel_first).await.unwrap().status,
        "failed"
    );
    assert_eq!(
        repo.cancel(&p.id, cancel_first, 12)
            .await
            .unwrap()
            .finished_at_ms,
        Some(10)
    );

    repo.complete(&p.id, complete_first, vec![], 20)
        .await
        .unwrap();
    let terminal = repo.cancel(&p.id, complete_first, 21).await.unwrap();
    assert_eq!(terminal.status, "succeeded");
    assert_eq!(terminal.finished_at_ms, Some(20));
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
    FaultInjector::for_event_bus(&app.state.events)
        .fail_next_for_session(FaultPoint::EventCommit, session.id);
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
async fn scoped_event_fault_does_not_cross_apps_with_the_same_session_id() {
    let first = TestApp::spawn().await;
    let second = TestApp::spawn().await;
    let p = principal();
    let first_session = first
        .state
        .session_repo
        .create(&p.id, "first", now_ms())
        .await
        .unwrap();
    let second_session = second
        .state
        .session_repo
        .create(&p.id, "second", now_ms())
        .await
        .unwrap();
    assert_eq!(first_session.id, second_session.id);
    let first_writer = EventWriter::new(
        p.id.clone(),
        first_session.id,
        first.state.message_repo.clone(),
        first.state.events.clone(),
    );
    let second_writer = EventWriter::new(
        p.id.clone(),
        second_session.id,
        second.state.message_repo.clone(),
        second.state.events.clone(),
    );

    FaultInjector::for_event_bus(&first.state.events)
        .fail_next_for_session(FaultPoint::EventCommit, first_session.id);

    second_writer
        .assistant("另一个应用不应消费故障")
        .await
        .unwrap();
    assert!(first_writer.assistant("目标应用消费故障").await.is_err());
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
    let app = TestApp::builder().agent_factory(factory).spawn().await;
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
