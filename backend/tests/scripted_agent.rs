#![cfg(feature = "e2e-test")]

mod common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::testing::{ScriptScenario, ScriptTurn, ScriptedAgentFactory};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::TestApp;

async fn create_session(app: &TestApp) -> String {
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(r#"{"title":"scripted"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn post_and_wait(app: &TestApp, session_id: &str, message: &str) -> StatusCode {
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{session_id}/messages"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({"message": message}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.state.scheduler.active_count() != 0 || app.state.scheduler.waiting_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scripted run finishes");
    status
}

async fn messages(app: &TestApp, session_id: &str) -> Vec<Value> {
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{session_id}/messages"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()["messages"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn scripted_tool_call_runs_registered_tool_and_persists_events() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "storage".into(),
        expected_prompt: Some("查看空间".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "call-storage".into(),
                name: "search_examples".into(),
                args: json!({}),
            },
            ScriptTurn::Text {
                text: "空间已查询".into(),
            },
        ],
    });
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    assert_eq!(
        post_and_wait(&app, &session, "查看空间").await,
        StatusCode::ACCEPTED
    );

    let events = messages(&app, &session).await;
    let statuses = events
        .iter()
        .filter(|event| event["type"] == "tool-call")
        .map(|event| event["status"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(statuses, ["running", "success"], "events: {events:#?}");
    assert!(
        events
            .iter()
            .any(|event| { event["type"] == "assistant" && event["content"] == "空间已查询" })
    );
    let trace = factory.trace();
    assert_eq!(trace[0].scenario_id, "storage");
    assert_eq!(trace[0].tool_names.len(), 16);
    assert!(trace[0].system_prompt.contains("# Engine resources"));
    assert_eq!(trace[0].calls.len(), 2);
}

#[tokio::test]
async fn unknown_tool_and_malformed_args_end_with_readable_error() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "unknown".into(),
        expected_prompt: None,
        turns: vec![ScriptTurn::ToolCall {
            call_id: "bad-name".into(),
            name: "does_not_exist".into(),
            args: json!({}),
        }],
    });
    factory.enqueue(ScriptScenario {
        id: "malformed".into(),
        expected_prompt: None,
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "bad-args".into(),
                name: "get_storage_usage".into(),
                args: json!("not-an-object"),
            },
            ScriptTurn::Text {
                text: "参数错误已处理".into(),
            },
        ],
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    for expected in ["unknown", "malformed"] {
        let session = create_session(&app).await;
        assert_eq!(
            post_and_wait(&app, &session, expected).await,
            StatusCode::ACCEPTED
        );
        let events = messages(&app, &session).await;
        assert!(events.iter().any(|event| event["type"] == "assistant"));
    }
}

#[tokio::test]
async fn missing_skill_is_a_failed_tool_result() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "skill-gate".into(),
        expected_prompt: None,
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "script".into(),
                name: "create_script".into(),
                args: json!({"name":"flow.rss","content":"result = 1"}),
            },
            ScriptTurn::Text {
                text: "请先加载技能".into(),
            },
        ],
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    let session = create_session(&app).await;
    post_and_wait(&app, &session, "运行潮流").await;
    let events = messages(&app, &session).await;
    assert!(
        events.iter().any(|event| {
            event["type"] == "tool-call"
                && event["toolName"] == "create_script"
                && event["status"] == "failed"
        }),
        "events: {events:#?}"
    );
}

#[tokio::test]
async fn provider_error_releases_scheduler_permit() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "provider-error".into(),
        expected_prompt: None,
        turns: vec![ScriptTurn::Error {
            message: "provider unavailable".into(),
        }],
    });
    factory.enqueue(ScriptScenario {
        id: "retry".into(),
        expected_prompt: None,
        turns: vec![ScriptTurn::Text {
            text: "重试成功".into(),
        }],
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    let session = create_session(&app).await;
    assert_eq!(
        post_and_wait(&app, &session, "失败一次").await,
        StatusCode::ACCEPTED
    );
    assert_eq!(app.state.scheduler.active_count(), 0);
    assert_eq!(
        post_and_wait(&app, &session, "重试").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert!(
        events.iter().any(|event| {
            event["type"] == "assistant" && event["content"] == "agent 运行失败，请稍后重试"
        }),
        "events: {events:#?}"
    );
    assert!(events.iter().any(|event| event["content"] == "重试成功"));
}

#[tokio::test]
async fn barrier_holds_and_releases_agent_run_without_sleep() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "barrier".into(),
        expected_prompt: None,
        turns: vec![
            ScriptTurn::DelayUntilReleased {
                barrier_id: "llm-ready".into(),
            },
            ScriptTurn::Text {
                text: "屏障已释放".into(),
            },
        ],
    });
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{session}/messages"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(r#"{"message":"等待"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            factory.wait_until_blocked("llm-ready")
        )
        .await
        .expect("barrier reached")
    );
    assert_eq!(app.state.scheduler.active_count(), 1);
    assert!(factory.release("llm-ready"));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.state.scheduler.active_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("barrier run finishes");
    assert!(
        messages(&app, &session)
            .await
            .iter()
            .any(|event| { event["type"] == "assistant" && event["content"] == "屏障已释放" })
    );
}
