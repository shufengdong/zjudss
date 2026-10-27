#![cfg(feature = "e2e-test")]

mod common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::PublishDraftRequest;
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;
use zjudss::testing::{ScriptScenario, ScriptTurn, ScriptedAgentFactory};

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

async fn runs(app: &TestApp, session_id: &str) -> Vec<Value> {
    let cookie = app.login_cookie("instance").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{session_id}/runs"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()["runs"]
        .as_array()
        .unwrap()
        .clone()
}

async fn apply_csv(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    table_name: &str,
    csv: &[u8],
    expected: Option<DraftRef>,
    sequence: usize,
) -> DraftRef {
    let client_id = format!("agent-study-{session_id}-{table_name}-{sequence}");
    common::publish_model_bytes(
        app,
        user,
        session_id,
        &client_id,
        &format!("{table_name}.csv"),
        csv,
        expected,
        Some(table_name),
    )
    .await
}

async fn seed_canonical_draft(app: &TestApp, user: &CurrentUser, session_id: i32) -> DraftRef {
    let client_id = format!("agent-study-{session_id}-canonical");
    common::publish_model_bytes(
        app,
        user,
        session_id,
        &client_id,
        "master.dss",
        b"New Circuit.Test\n",
        None,
        None,
    )
    .await
}

async fn publish_agent_study_model(app: &TestApp, session_id: i32) {
    let user = common::test_user("instance");
    let tables: [(&str, &[u8]); 8] = [
        ("bus", b"id\nsource\nb1\n"),
        ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
        ("voltage_source", b"name,bus\nsource,source\n"),
        ("motor", b"name,bus,motor_marker\nm1,b1,present\n"),
        ("spectrum", b"name,harmonic\ndefault,1\n"),
        ("fuse", b"name\n"),
        ("recloser", b"name\n"),
        ("relay", b"name\n"),
    ];
    let mut draft = Some(seed_canonical_draft(app, &user, session_id).await);
    for (sequence, (table_name, csv)) in tables.into_iter().enumerate() {
        draft = Some(apply_csv(app, &user, session_id, table_name, csv, draft, sequence).await);
    }
    let draft = draft.unwrap();
    app.state
        .version_service
        .publish(
            &user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: format!("publish-agent-study-{session_id}"),
                description: None,
            },
        )
        .await
        .unwrap();
}

fn tool_names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| event["type"] == "tool-call" && event["status"] == "running")
        .filter_map(|event| event["toolName"].as_str())
        .collect()
}

fn assert_no_script_or_run(events: &[Value]) {
    assert!(
        events
            .iter()
            .all(|event| event["type"] != "script" && event["type"] != "simulation-run"),
        "events: {events:#?}"
    );
}

#[tokio::test]
async fn study_power_flow_searches_then_prepares_a_configuration_without_a_script() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    publish_agent_study_model(&app, session.parse().unwrap()).await;
    factory.enqueue(ScriptScenario {
        id: "study-power-flow-config".into(),
        expected_prompt: Some("求解牛顿法潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall { call_id: "inspect".into(), name: "inspect_workspace".into(), args: json!({}) },
            ScriptTurn::ToolCall { call_id: "load-planning".into(), name: "load_agent_skill".into(), args: json!({"skill_id":"study-planning"}) },
            ScriptTurn::ToolCall { call_id: "search".into(), name: "search_study_facades".into(), args: json!({"query":"牛顿法潮流","explicitFunction":"ds_newton_raphson_pf"}) },
            ScriptTurn::ToolCall { call_id: "prepare".into(), name: "prepare_study".into(), args: json!({"facadeId":"static_power_flow_newton","intent":{"request":"求解牛顿法潮流"}}) },
            ScriptTurn::Text { text: "研究配置已准备，请在卡片中确认输入、参数和结果。".into() },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &session, "求解牛顿法潮流").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        [
            "inspect_workspace",
            "load_agent_skill",
            "search_study_facades",
            "prepare_study"
        ]
    );
    assert_no_script_or_run(&events);
    assert!(runs(&app, &session).await.is_empty());
}

#[tokio::test]
async fn prepared_configuration_survives_provider_finalization_failure() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    publish_agent_study_model(&app, session.parse().unwrap()).await;
    factory.enqueue(ScriptScenario {
        id: "prepared-config-provider-failure".into(),
        expected_prompt: Some("求解牛顿法潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "prepare".into(),
                name: "prepare_study".into(),
                args: json!({"facadeId":"static_power_flow_newton","intent":{"request":"求解牛顿法潮流"}}),
            },
            ScriptTurn::Error {
                message: "provider closed after tool success".into(),
            },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &session, "求解牛顿法潮流").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_no_script_or_run(&events);
    assert!(events.iter().any(|event| {
        event["type"] == "assistant"
            && event["content"]
                .as_str()
                .is_some_and(|text| text.contains("研究配置已准备"))
    }));
    assert!(!events.iter().any(|event| {
        event["type"] == "assistant" && event["content"] == "agent 运行失败，请稍后重试"
    }));
}

#[tokio::test]
async fn conversational_agent_cannot_create_a_generic_script() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    factory.enqueue(ScriptScenario {
        id: "generic-script-deferred".into(),
        expected_prompt: Some("保存通用脚本".into()),
        turns: vec![
            ScriptTurn::ToolCall { call_id: "create".into(), name: "create_script".into(), args: json!({"name":"generic.rss","content":"answer = select(col(value), source);"}) },
            ScriptTurn::Text { text: "对话 Agent 无权创建脚本，请使用研究配置卡。".into() },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &session, "保存通用脚本").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_no_script_or_run(&events);
    assert!(runs(&app, &session).await.is_empty());
    assert_eq!(
        app.state
            .script_service
            .list(&common::test_user("instance"), session_id)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn ambiguous_study_requests_present_catalog_choices_before_prepare() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    factory.enqueue(ScriptScenario {
        id: "study-choice".into(),
        expected_prompt: Some("计算潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "search".into(),
                name: "search_study_facades".into(),
                args: json!({"query":"潮流"}),
            },
            ScriptTurn::Text {
                text: "可选径向潮流和牛顿潮流；请确认所需方法。".into(),
            },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &session, "计算潮流").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        ["load_agent_skill", "search_study_facades"]
    );
    assert_no_script_or_run(&events);
    assert!(events.iter().any(|event| {
        event["type"] == "assistant"
            && event["content"]
                .as_str()
                .is_some_and(|text| text.contains("径向潮流") && text.contains("牛顿潮流"))
    }));
}

#[tokio::test]
async fn transient_studies_use_catalog_search_and_configuration_cards() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    factory.enqueue(ScriptScenario {
        id: "motor-config".into(),
        expected_prompt: Some("模拟电机启动暂态".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "search".into(),
                name: "search_study_facades".into(),
                args: json!({"query":"电机 暂态","explicitFunction":"ds_motor_dynamics"}),
            },
            ScriptTurn::ToolCall {
                call_id: "prepare".into(),
                name: "prepare_study".into(),
                args: json!({"facadeId":"motor_dynamics","intent":{"request":"模拟电机启动暂态"}}),
            },
            ScriptTurn::Text {
                text: "电机动态配置已准备，请补齐数据和参数后确认。".into(),
            },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &session, "模拟电机启动暂态").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        ["load_agent_skill", "search_study_facades", "prepare_study"]
    );
    assert_no_script_or_run(&events);
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
