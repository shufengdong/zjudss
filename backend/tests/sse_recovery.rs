mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use futures::StreamExt;
use tower::ServiceExt;
use zjudss::application::{
    EventDiagnostic, EventWriter, StudyConfigCard, UploadBatchCard, UploadCandidateCard,
    WorkflowProgressCard, now_ms,
};
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
            .list_after(&other_scope, first_session.id, foreign.id, 1)
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

#[tokio::test]
async fn durable_operation_messages_upsert_by_stable_id_and_replay_each_revision() {
    let app = TestApp::spawn().await;
    let scope = instance_scope();
    let session = app
        .state
        .session_repo
        .create(&scope, "durable", now_ms())
        .await
        .unwrap();
    let writer = EventWriter::new(
        scope.clone(),
        session.id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );
    let accepted = writer
        .upload_batch(UploadBatchCard {
            batch_id: "batch-7".into(),
            batch_revision: 1,
            state: "accepted".into(),
            candidates: vec![UploadCandidateCard {
                id: "candidate-1".into(),
                name: "load.csv".into(),
                state: "pending".into(),
            }],
            model_transaction_state: "pending".into(),
            resource_transaction_state: "pending".into(),
            diagnostic: None,
        })
        .await
        .unwrap();
    let later_message = writer.assistant("上传批次仍在处理").await.unwrap();
    let ready = writer
        .upload_batch(UploadBatchCard {
            batch_id: "batch-7".into(),
            batch_revision: 2,
            state: "confirmed".into(),
            candidates: vec![UploadCandidateCard {
                id: "candidate-1".into(),
                name: "load.csv".into(),
                state: "ready".into(),
            }],
            model_transaction_state: "succeeded".into(),
            resource_transaction_state: "succeeded".into(),
            diagnostic: Some(EventDiagnostic {
                code: "normalized".into(),
                message: "文件已规范化".into(),
            }),
        })
        .await
        .unwrap();
    let delayed = writer
        .upload_batch(UploadBatchCard {
            batch_id: "batch-7".into(),
            batch_revision: 1,
            state: "accepted".into(),
            candidates: vec![UploadCandidateCard {
                id: "candidate-1".into(),
                name: "load.csv".into(),
                state: "pending".into(),
            }],
            model_transaction_state: "pending".into(),
            resource_transaction_state: "pending".into(),
            diagnostic: None,
        })
        .await
        .unwrap();
    assert_eq!(accepted.message_id(), ready.message_id());
    assert_eq!(ready.message_id(), delayed.message_id());
    assert_eq!(accepted.revision(), 1);
    assert_eq!(ready.revision(), 2);
    assert_eq!(delayed.revision(), 2);
    let stored = app
        .state
        .message_repo
        .list_by_session(&scope, session.id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    let operation = stored
        .iter()
        .find(|message| message.operation_id.as_deref() == Some("batch-7"))
        .unwrap();
    assert_eq!(operation.revision, 2);
    assert!(
        operation
            .payload_json
            .as_deref()
            .unwrap()
            .contains("confirmed")
    );
    assert!(
        !operation
            .payload_json
            .as_deref()
            .unwrap()
            .contains("stderr")
    );

    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{}/events", session.id))
                .header(
                    "last-event-id",
                    format!("{}:1", later_message.message_id().unwrap()),
                )
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let output = read_events(response.into_body(), 1).await;
    assert!(output.contains(&format!("id: {}:2", ready.message_id().unwrap())));
    assert!(output.contains("upload-batch"));
}

#[tokio::test]
async fn delayed_configuration_and_workflow_cards_cannot_regress_current_facts() {
    let app = TestApp::spawn().await;
    let scope = instance_scope();
    let session = app
        .state
        .session_repo
        .create(&scope, "monotonic", now_ms())
        .await
        .unwrap();
    let writer = EventWriter::new(
        scope.clone(),
        session.id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );

    let configuration = writer
        .study_config(StudyConfigCard {
            configuration_id: "configuration-1".into(),
            revision: 2,
            state: "ready_to_confirm".into(),
            action_card: serde_json::json!({"revision":2}),
        })
        .await
        .unwrap();
    let delayed_configuration = writer
        .study_config(StudyConfigCard {
            configuration_id: "configuration-1".into(),
            revision: 1,
            state: "collecting_inputs".into(),
            action_card: serde_json::json!({"revision":1}),
        })
        .await
        .unwrap();
    assert_eq!(configuration.revision(), 1);
    assert_eq!(delayed_configuration.revision(), 1);

    let workflow = writer
        .workflow_progress(WorkflowProgressCard {
            workflow_id: "workflow-1".into(),
            state: "succeeded".into(),
            execution_index: 1,
            execution_max: 2,
            repair_index: 0,
            repair_max: 3,
            elapsed_ms: 10,
            attempts: vec![],
            script_id: Some("script-1".into()),
            output_dataset_ids: vec!["dataset-1".into()],
            failure_code: None,
        })
        .await
        .unwrap();
    let conflicting_terminal = writer
        .workflow_progress(WorkflowProgressCard {
            workflow_id: "workflow-1".into(),
            state: "failed".into(),
            execution_index: 1,
            execution_max: 2,
            repair_index: 0,
            repair_max: 3,
            elapsed_ms: 11,
            attempts: vec![],
            script_id: None,
            output_dataset_ids: vec![],
            failure_code: Some("late_failure".into()),
        })
        .await
        .unwrap();
    assert_eq!(workflow.revision(), 1);
    assert_eq!(conflicting_terminal.revision(), 1);

    let stored = app
        .state
        .message_repo
        .list_by_session(&scope, session.id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().any(|message| {
        message.kind == "study-config"
            && message
                .payload_json
                .as_deref()
                .is_some_and(|payload| payload.contains("ready_to_confirm"))
    }));
    assert!(stored.iter().any(|message| {
        message.kind == "workflow-progress"
            && message
                .payload_json
                .as_deref()
                .is_some_and(|payload| payload.contains("succeeded"))
    }));
}
