#![cfg(feature = "e2e-test")]

mod common;

use std::io::Cursor;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use diesel::sql_types::Integer;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::study_planning::{
    MotorDynamicsRequest, SolverPreference, StudyRequest, TransientSubtype,
};
use zjudss::application::{PublishDraftRequest, UploadBatch, UploadFile};
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;
use zjudss::storage::StageLimits;
use zjudss::testing::{ScriptScenario, ScriptTurn, ScriptedAgentFactory};
use zjudss::tools::ReadyStudyPlanRegistry;

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
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(csv), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            user,
            session_id,
            client_id.clone(),
            expected,
            UploadBatch {
                client_batch_id: client_id.clone(),
                files: vec![UploadFile {
                    client_file_id: client_id,
                    file_name: format!("{table_name}.csv"),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let import_id = accepted.id.parse().unwrap();
    let ready = app
        .state
        .model_import_service
        .process(user, import_id)
        .await
        .unwrap();
    let candidate = ready
        .candidates
        .into_iter()
        .find(|candidate| candidate.table_name.as_deref() == Some(table_name))
        .unwrap();
    app.state
        .model_import_service
        .resolve_and_apply(user, import_id, expected, vec![candidate.id])
        .await
        .unwrap()
}

async fn seed_canonical_draft(app: &TestApp, user: &CurrentUser, session_id: i32) -> DraftRef {
    let client_id = format!("agent-study-{session_id}-canonical");
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(
            Cursor::new(b"New Circuit.Test\n".to_vec()),
            StageLimits::default(),
        )
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            user,
            session_id,
            client_id.clone(),
            None,
            UploadBatch {
                client_batch_id: client_id.clone(),
                files: vec![UploadFile {
                    client_file_id: client_id,
                    file_name: "master.dss".into(),
                    relative_path: Some("master.dss".into()),
                    media_type: Some("text/plain".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let import_id = accepted.id.parse().unwrap();
    let ready = app
        .state
        .model_import_service
        .process(user, import_id)
        .await
        .unwrap();
    app.state
        .model_import_service
        .resolve_and_apply(
            user,
            import_id,
            None,
            ready
                .candidates
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
        )
        .await
        .unwrap()
}

async fn publish_agent_study_model(app: &TestApp, session_id: i32) {
    let user = common::test_user("instance");
    let tables: [(&str, &[u8]); 5] = [
        ("bus", b"id\nsource\nb1\n"),
        ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
        ("voltage_source", b"name,bus\nsource,source\n"),
        ("motor", b"name,bus,motor_marker\nm1,b1,present\n"),
        ("spectrum", b"name,harmonic\ndefault,1\n"),
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
            },
        )
        .await
        .unwrap();
}

#[derive(diesel::QueryableByName)]
struct MessageIdRow {
    #[diesel(sql_type = Integer)]
    id: i32,
}

async fn next_message_id(app: &TestApp) -> i32 {
    let pool = app.pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().unwrap();
        diesel::sql_query("SELECT COALESCE(MAX(id), 0) + 1 AS id FROM messages")
            .get_result::<MessageIdRow>(&mut conn)
            .unwrap()
            .id
    })
    .await
    .unwrap()
}

async fn ready_plan_id(app: &TestApp, session_id: i32, request: StudyRequest) -> String {
    let plan = app
        .state
        .study_planning_service
        .plan(&common::test_user("instance"), session_id, request)
        .await
        .unwrap();
    let mut registry = ReadyStudyPlanRegistry::default();
    registry.insert(next_message_id(app).await, plan.ready.unwrap())
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

const POWER_FLOW_SCRIPT: &str = r#"[bus_phase, summary] = ds_radial_pf(
    bus, line, linecode, load, transformer, voltage_source, motor,
    transformer_winding, transformer_impedance, current_source, generator, ibr,
    switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para
);"#;

const IMPORTING_POWER_FLOW_SCRIPT: &str = r#"[
    bus, line, linecode, load, transformer, voltage_source, motor,
    transformer_winding, transformer_impedance, current_source, generator, ibr,
    switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para
] = import_dss("case/model.dss");
[bus_phase, summary] = ds_radial_pf(
    bus, line, linecode, load, transformer, voltage_source, motor,
    transformer_winding, transformer_impedance, current_source, generator, ibr,
    switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para
);"#;

const MOTOR_DYNAMICS_SCRIPT: &str = r#"[motor_state] = ds_motor_dynamics(
    bus, line, linecode, load, transformer, voltage_source, motor,
    transformer_winding, transformer_impedance, current_source, generator, ibr,
    switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage,
    timeline, event, tcc_curve, fuse, recloser, relay, spec, para
);"#;

#[tokio::test]
async fn study_power_flow_uses_ready_plan_before_creating_the_planned_script() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    publish_agent_study_model(&app, session_id).await;
    let request = StudyRequest::StaticPowerFlow {
        solver_preference: SolverPreference::Auto,
        requested_results: vec![],
    };
    let plan_id = ready_plan_id(&app, session_id, request.clone()).await;
    factory.enqueue(ScriptScenario {
        id: "study-power-flow-ready".into(),
        expected_prompt: Some("计算潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "load-planning".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "plan".into(),
                name: "plan_study".into(),
                args: json!({"request": request}),
            },
            ScriptTurn::ToolCall {
                call_id: "load-rss".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"rss-generation"}),
            },
            ScriptTurn::ToolCall {
                call_id: "search-pf-docs".into(),
                name: "search_engine_docs".into(),
                args: json!({"query":"ds_radial_pf"}),
            },
            ScriptTurn::ToolCall {
                call_id: "create".into(),
                name: "create_script".into(),
                args: json!({
                    "name":"power-flow.rss",
                    "content":POWER_FLOW_SCRIPT,
                    "studyPlanId":plan_id
                }),
            },
            ScriptTurn::Text {
                text: "已按当前模型的径向潮流计划生成脚本，请检查后点击运行。".into(),
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
        [
            "inspect_workspace",
            "load_agent_skill",
            "plan_study",
            "load_agent_skill",
            "search_engine_docs",
            "create_script"
        ]
    );
    let script = events
        .iter()
        .find(|event| event["type"] == "script")
        .expect("planned script message");
    assert!(script.to_string().contains("ds_radial_pf"));
    assert!(!script.to_string().contains("ds_newton_raphson_pf"));
    assert!(runs(&app, &session).await.is_empty());
}

#[tokio::test]
async fn planned_agent_script_cannot_reimport_the_active_model() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    publish_agent_study_model(&app, session_id).await;
    let request = StudyRequest::StaticPowerFlow {
        solver_preference: SolverPreference::Auto,
        requested_results: vec![],
    };
    let plan_id = ready_plan_id(&app, session_id, request.clone()).await;
    factory.enqueue(ScriptScenario {
        id: "study-power-flow-reimport-rejected".into(),
        expected_prompt: Some("计算潮流但错误地重新导入".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "inspect-reimport".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "load-planning-reimport".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "plan-reimport".into(),
                name: "plan_study".into(),
                args: json!({"request":request}),
            },
            ScriptTurn::ToolCall {
                call_id: "load-rss-reimport".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"rss-generation"}),
            },
            ScriptTurn::ToolCall {
                call_id: "create-reimport".into(),
                name: "create_script".into(),
                args: json!({
                    "name":"reimport.rss",
                    "content":IMPORTING_POWER_FLOW_SCRIPT,
                    "studyPlanId":plan_id
                }),
            },
            ScriptTurn::Text {
                text: "脚本被静态校验拒绝，需要直接使用 active 模型输入。".into(),
            },
        ],
    });

    assert_eq!(
        post_and_wait(&app, &session, "计算潮流但错误地重新导入").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_no_script_or_run(&events);
    assert!(
        events.iter().any(|event| {
            event["type"] == "tool-call"
                && event["toolName"] == "create_script"
                && event["status"] == "failed"
                && event.to_string().contains("inputAliases")
        }),
        "events: {events:#?}"
    );
    assert!(
        app.state
            .script_service
            .list(&common::test_user("instance"), session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn incomplete_study_requests_ask_once_with_every_reported_missing_field() {
    struct Case {
        prompt: &'static str,
        request: Value,
        answer: &'static str,
        fields: &'static [&'static str],
    }
    let cases = [
        Case {
            prompt: "做一天时序模拟",
            request: json!({"studyType":"qsts"}),
            answer: "还需要确认 mode、startSeconds、stepSizeSeconds、number 和 profile。",
            fields: &[
                "mode",
                "startSeconds",
                "stepSizeSeconds",
                "number",
                "profile",
            ],
        },
        Case {
            prompt: "短路计算",
            request: json!({"studyType":"fault"}),
            answer: "还需要一次性确认 cases 和 allowIdealSources。",
            fields: &["cases", "allowIdealSources"],
        },
        Case {
            prompt: "谐波分析",
            request: json!({"studyType":"harmonic"}),
            answer: "还需要 hMax；当前模型还缺少 model.spectrum 引用。",
            fields: &["hMax", "model.spectrum"],
        },
        Case {
            prompt: "暂态仿真",
            request: json!({"studyType":"transient"}),
            answer: "请先确认 subtype；当前仅支持 motor_dynamics。",
            fields: &["subtype"],
        },
    ];
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    publish_agent_study_model(&app, session.parse().unwrap()).await;

    for case in &cases {
        factory.enqueue(ScriptScenario {
            id: format!("study-clarify-{}", case.prompt),
            expected_prompt: Some(case.prompt.into()),
            turns: vec![
                ScriptTurn::ToolCall {
                    call_id: format!("inspect-{}", case.prompt),
                    name: "inspect_workspace".into(),
                    args: json!({}),
                },
                ScriptTurn::ToolCall {
                    call_id: format!("skill-{}", case.prompt),
                    name: "load_agent_skill".into(),
                    args: json!({"skill_id":"study-planning"}),
                },
                ScriptTurn::ToolCall {
                    call_id: format!("plan-{}", case.prompt),
                    name: "plan_study".into(),
                    args: json!({"request":case.request}),
                },
                ScriptTurn::Text {
                    text: case.answer.into(),
                },
            ],
        });
        assert_eq!(
            post_and_wait(&app, &session, case.prompt).await,
            StatusCode::ACCEPTED
        );
        let events = messages(&app, &session).await;
        assert_no_script_or_run(&events);
        assert!(runs(&app, &session).await.is_empty());
        assert_eq!(
            tool_names(&events)
                .into_iter()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>(),
            ["inspect_workspace", "load_agent_skill", "plan_study"]
        );
        let answer = events
            .iter()
            .rev()
            .find(|event| event["type"] == "assistant")
            .unwrap()["content"]
            .as_str()
            .unwrap();
        for field in case.fields {
            assert!(answer.contains(field), "missing {field} in {answer}");
        }
    }
}

#[tokio::test]
async fn transient_study_supports_motor_dynamics_and_rejects_emt() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    publish_agent_study_model(&app, session_id).await;
    let motor_request = StudyRequest::Transient {
        subtype: Some(TransientSubtype::MotorDynamics),
        motor_dynamics: Some(MotorDynamicsRequest {
            duration_seconds: Some(2.0),
            step_size_seconds: Some(0.01),
            events: Some(vec![]),
        }),
        requested_results: vec![],
    };
    let plan_id = ready_plan_id(&app, session_id, motor_request.clone()).await;
    factory.enqueue(ScriptScenario {
        id: "study-motor-dynamics-ready".into(),
        expected_prompt: Some("模拟电机启动暂态2秒，步长0.01秒，无外部事件".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "motor-inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "motor-planning-skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "motor-plan".into(),
                name: "plan_study".into(),
                args: json!({"request":motor_request}),
            },
            ScriptTurn::ToolCall {
                call_id: "motor-rss-skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"rss-generation"}),
            },
            ScriptTurn::ToolCall {
                call_id: "search-motor-docs".into(),
                name: "search_engine_docs".into(),
                args: json!({"query":"ds_motor_dynamics"}),
            },
            ScriptTurn::ToolCall {
                call_id: "motor-create".into(),
                name: "create_script".into(),
                args: json!({
                    "name":"motor-dynamics.rss",
                    "content":MOTOR_DYNAMICS_SCRIPT,
                    "studyPlanId":plan_id
                }),
            },
            ScriptTurn::Text {
                text: "已生成电机—网络动态脚本，请检查后点击运行。".into(),
            },
        ],
    });
    assert_eq!(
        post_and_wait(
            &app,
            &session,
            "模拟电机启动暂态2秒，步长0.01秒，无外部事件"
        )
        .await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    let script = events
        .iter()
        .find(|event| event["type"] == "script")
        .expect("motor dynamics script");
    assert!(script.to_string().contains("ds_motor_dynamics"));
    assert!(runs(&app, &session).await.is_empty());

    let emt_session = create_session(&app).await;
    publish_agent_study_model(&app, emt_session.parse().unwrap()).await;
    factory.enqueue(ScriptScenario {
        id: "study-emt-unsupported".into(),
        expected_prompt: Some("做EMT暂态仿真".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "emt-inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "emt-planning-skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "emt-plan".into(),
                name: "plan_study".into(),
                args: json!({"request":{"studyType":"transient","subtype":"emt"}}),
            },
            ScriptTurn::Text {
                text: "当前引擎不支持 EMT 暂态仿真；当前暂态只支持 motor_dynamics。".into(),
            },
        ],
    });
    assert_eq!(
        post_and_wait(&app, &emt_session, "做EMT暂态仿真").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &emt_session).await;
    assert_no_script_or_run(&events);
    assert!(runs(&app, &emt_session).await.is_empty());
    assert!(
        events.iter().rev().any(|event| {
            event["type"] == "assistant"
                && event["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("不支持") && text.contains("motor_dynamics"))
        }),
        "events: {events:#?}"
    );
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
    assert_eq!(trace[0].tool_names.len(), 18);
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
            ScriptTurn::Error {
                message: "provider unavailable after failed tool".into(),
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
    assert!(
        events.iter().any(|event| {
            event["type"] == "assistant" && event["content"] == "agent 运行失败，请稍后重试"
        }),
        "events: {events:#?}"
    );
}

#[tokio::test]
async fn turn_limit_failure_explains_that_no_script_was_created() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "turn-limit".into(),
        expected_prompt: Some("计算潮流".into()),
        turns: (0..8)
            .map(|index| ScriptTurn::ToolCall {
                call_id: format!("storage-{index}"),
                name: "get_storage_usage".into(),
                args: json!({}),
            })
            .collect(),
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    let session = create_session(&app).await;

    assert_eq!(
        post_and_wait(&app, &session, "计算潮流").await,
        StatusCode::ACCEPTED
    );

    let events = messages(&app, &session).await;
    assert!(
        events.iter().any(|event| {
            event["type"] == "assistant"
                && event["content"] == "本轮工具调用已达到上限，尚未生成脚本，请重新发送请求"
        }),
        "events: {events:#?}"
    );
}

#[tokio::test]
async fn successful_script_creation_is_not_overwritten_by_a_later_provider_error() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    factory.enqueue(ScriptScenario {
        id: "script-created-before-provider-error".into(),
        expected_prompt: Some("计算潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "load-rss-skill".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"rss-generation"}),
            },
            ScriptTurn::ToolCall {
                call_id: "create-flow-script".into(),
                name: "create_script".into(),
                args: json!({"name":"power-flow.rss","content":"result = 1"}),
            },
            ScriptTurn::Error {
                message: "provider unavailable after tool success".into(),
            },
        ],
    });
    let app = TestApp::spawn_with_agent_factory(factory).await;
    let session = create_session(&app).await;

    assert_eq!(
        post_and_wait(&app, &session, "计算潮流").await,
        StatusCode::ACCEPTED
    );

    let events = messages(&app, &session).await;
    let script_index = events
        .iter()
        .position(|event| event["type"] == "script")
        .expect("script message");
    let tool_success_index = events
        .iter()
        .position(|event| {
            event["type"] == "tool-call"
                && event["toolName"] == "create_script"
                && event["status"] == "success"
        })
        .expect("successful create_script result");
    let assistants = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event["type"] == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(assistants.len(), 1, "events: {events:#?}");
    assert_eq!(
        assistants[0].1["content"], "脚本已生成，请检查代码并点击脚本卡片“运行”",
        "events: {events:#?}"
    );
    assert!(
        script_index < tool_success_index && tool_success_index < assistants[0].0,
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
