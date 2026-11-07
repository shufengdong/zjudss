#![cfg(feature = "e2e-test")]

mod common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::{RunQueryDsl, sql_query};
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::{
    CandidateDecision, CandidatePurpose, ConfirmStudyRequest, EventWriter, PrepareStudyRequest,
    PublishDraftRequest, StudyConfigCard, StudyConfigurationState,
};
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

async fn seed_agent_study_draft(app: &TestApp, session_id: i32) -> DraftRef {
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
    draft.unwrap()
}

async fn publish_agent_study_model(app: &TestApp, session_id: i32) {
    let user = common::test_user("instance");
    let draft = seed_agent_study_draft(app, session_id).await;
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

async fn import_agent_study_case(app: &TestApp, user: &CurrentUser, session_id: i32) {
    let operation_id = format!("agent-result-selection-{session_id}");
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let batch = app
        .state
        .case_import_service
        .apply(
            user,
            session_id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            operation_id.clone(),
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: (candidate.purpose == Some(CandidatePurpose::ModelTable))
                .then(|| candidate.proposed_alias.clone().unwrap()),
            display_name: None,
            alias: None,
        })
        .collect();
    app.state
        .upload_confirmation_service
        .confirm(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            None,
            &operation_id,
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
async fn explicit_publish_request_uses_exact_draft_and_activates_the_version() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    let draft = seed_canonical_draft(&app, &common::test_user("instance"), session_id).await;
    factory.enqueue(ScriptScenario {
        id: "explicit-model-publish".into(),
        expected_prompt: Some("发布当前模型草稿，描述为案例导入版本".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "load-intake".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"model-intake"}),
            },
            ScriptTurn::ToolCall {
                call_id: "inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "publish".into(),
                name: "publish_model_version".into(),
                args: json!({
                    "expectedDraft":{"id":draft.id,"revision":draft.revision},
                    "description":"案例导入版本"
                }),
            },
            ScriptTurn::Text {
                text: "当前模型草稿已发布并自动激活。".into(),
            },
        ],
    });

    assert_eq!(
        post_and_wait(&app, &session, "发布当前模型草稿，描述为案例导入版本").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        [
            "load_agent_skill",
            "inspect_workspace",
            "publish_model_version"
        ]
    );
    let workspace = app
        .state
        .version_service
        .workspace(&common::test_user("instance"), session_id)
        .await
        .unwrap();
    assert!(workspace.draft.is_none());
    assert_eq!(
        workspace.active.unwrap().version.description,
        "案例导入版本"
    );
    assert_no_script_or_run(&events);
}

#[tokio::test]
async fn publish_then_study_request_continues_to_prepare_configuration_without_bypassing_confirmation()
 {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    let draft = seed_agent_study_draft(&app, session_id).await;
    factory.enqueue(ScriptScenario {
        id: "publish-then-study".into(),
        expected_prompt: Some("发布当前模型版本，然后使用 ds_newton_raphson_pf 准备潮流研究".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "load-intake".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"model-intake"}),
            },
            ScriptTurn::ToolCall {
                call_id: "inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "publish".into(),
                name: "publish_model_version".into(),
                args: json!({"expectedDraft":{"id":draft.id,"revision":draft.revision}}),
            },
            ScriptTurn::ToolCall {
                call_id: "load-planning".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "search".into(),
                name: "search_study_facades".into(),
                args: json!({"query":"潮流研究","explicitFunction":"ds_newton_raphson_pf"}),
            },
            ScriptTurn::ToolCall {
                call_id: "prepare".into(),
                name: "prepare_study".into(),
                args: json!({
                    "facadeId":"static_power_flow_newton",
                    "intent":{"request":"潮流研究"},
                    "explanation":"您要求在刚发布的模型上准备牛顿潮流研究。将使用目录中的牛顿-拉夫逊方法，并请您核对模型输入、参数和计划结果；配置完成后仍需点击确认按钮。"
                }),
            },
            ScriptTurn::Text {
                text: "版本已发布，研究配置已准备，请在卡片中最终确认。".into(),
            },
        ],
    });

    assert_eq!(
        post_and_wait(
            &app,
            &session,
            "发布当前模型版本，然后使用 ds_newton_raphson_pf 准备潮流研究",
        )
        .await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        [
            "load_agent_skill",
            "inspect_workspace",
            "publish_model_version",
            "load_agent_skill",
            "search_study_facades",
            "prepare_study"
        ]
    );
    assert_no_script_or_run(&events);
    assert!(runs(&app, &session).await.is_empty());
    let workspace = app
        .state
        .version_service
        .workspace(&common::test_user("instance"), session_id)
        .await
        .unwrap();
    assert!(workspace.active.is_some());
    assert!(workspace.draft.is_none());
}

#[tokio::test]
async fn script_or_result_generation_words_do_not_implicitly_publish_a_draft() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    let draft = seed_canonical_draft(&app, &common::test_user("instance"), session_id).await;
    factory.enqueue(ScriptScenario {
        id: "generation-is-not-publication".into(),
        expected_prompt: Some("根据当前草稿生成研究脚本和结果".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "load-intake".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"model-intake"}),
            },
            ScriptTurn::ToolCall {
                call_id: "inspect".into(),
                name: "inspect_workspace".into(),
                args: json!({}),
            },
            ScriptTurn::ToolCall {
                call_id: "unauthorized-publish".into(),
                name: "publish_model_version".into(),
                args: json!({"expectedDraft":{"id":draft.id,"revision":draft.revision}}),
            },
            ScriptTurn::Text {
                text: "需要先明确发布模型版本，并在研究配置卡中确认后才能生成脚本。".into(),
            },
        ],
    });

    assert_eq!(
        post_and_wait(&app, &session, "根据当前草稿生成研究脚本和结果").await,
        StatusCode::ACCEPTED
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        [
            "load_agent_skill",
            "inspect_workspace",
            "publish_model_version"
        ]
    );
    assert!(
        serde_json::to_string(&events)
            .unwrap()
            .contains("publish_authorization_required")
    );
    assert_no_script_or_run(&events);
    let workspace = app
        .state
        .version_service
        .workspace(&common::test_user("instance"), session_id)
        .await
        .unwrap();
    assert!(workspace.active.is_none());
    assert_eq!(workspace.draft.unwrap().draft.id, draft.id);
}

#[tokio::test]
async fn study_power_flow_searches_then_prepares_a_configuration_without_a_script() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    publish_agent_study_model(&app, session.parse().unwrap()).await;
    let explanation = "您希望用牛顿-拉夫逊法求解当前模型的静态潮流。该方法按您指定的算法处理网络；请核对模型输入和 para 参数，初始计划输出母线电压，之后仍需在卡片中确认。";
    factory.enqueue(ScriptScenario {
        id: "study-power-flow-config".into(),
        expected_prompt: Some("求解牛顿法潮流".into()),
        turns: vec![
            ScriptTurn::ToolCall { call_id: "inspect".into(), name: "inspect_workspace".into(), args: json!({}) },
            ScriptTurn::ToolCall { call_id: "load-planning".into(), name: "load_agent_skill".into(), args: json!({"skill_id":"study-planning"}) },
            ScriptTurn::ToolCall { call_id: "search".into(), name: "search_study_facades".into(), args: json!({"query":"牛顿法潮流","explicitFunction":"ds_newton_raphson_pf"}) },
            ScriptTurn::ToolCall { call_id: "prepare".into(), name: "prepare_study".into(), args: json!({"facadeId":"static_power_flow_newton","intent":{"request":"求解牛顿法潮流"},"explanation":explanation}) },
            ScriptTurn::Text { text: "PROVIDER_TAIL_SHOULD_NOT_APPEAR".into() },
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
    let configuration_index = events
        .iter()
        .position(|event| event["type"] == "study-config")
        .expect("study configuration card");
    let configuration_id = events[configuration_index]["card"]["configurationId"]
        .as_str()
        .expect("configuration id");
    let explanations = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event["type"] == "assistant"
                && event["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(explanation))
        })
        .collect::<Vec<_>>();
    assert_eq!(explanations.len(), 1);
    let (acknowledgement_index, acknowledgement) = explanations[0];
    let acknowledgement = acknowledgement["content"].as_str().unwrap();
    assert!(acknowledgement.contains("牛顿-拉夫逊潮流"));
    assert!(acknowledgement.contains(&format!("ID: {configuration_id}")));
    assert!(acknowledgement.contains("只有点击“确认并生成脚本”后才会开始执行"));
    assert!(acknowledgement_index < configuration_index);
    assert!(!events.iter().any(|event| {
        event["type"] == "assistant"
            && event["content"].as_str() == Some("PROVIDER_TAIL_SHOULD_NOT_APPEAR")
    }));
}

#[tokio::test]
async fn natural_language_result_selection_updates_the_durable_card_without_starting_work() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    let user = common::test_user("instance");
    import_agent_study_case(&app, &user, session_id).await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        configuration.status,
        StudyConfigurationState::ReadyToConfirm
    );
    let configuration_id = configuration.id.to_string();
    let action_card = serde_json::to_value(&configuration).unwrap();
    EventWriter::new(
        user.id.clone(),
        session_id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    )
    .study_config(StudyConfigCard {
        configuration_id: configuration_id.clone(),
        revision: configuration.revision,
        state: "ready_to_confirm".into(),
        action_card,
    })
    .await
    .unwrap();
    factory.enqueue(ScriptScenario {
        id: "natural-language-result-selection".into(),
        expected_prompt: Some("bus_voltage，summary".into()),
        turns: vec![
            ScriptTurn::ToolCall {
                call_id: "load-planning".into(),
                name: "load_agent_skill".into(),
                args: json!({"skill_id":"study-planning"}),
            },
            ScriptTurn::ToolCall {
                call_id: "update-results".into(),
                name: "update_study_results".into(),
                args: json!({
                    "configurationId": configuration_id.clone(),
                    "expectedRevision": configuration.revision,
                    "results": ["bus_voltage", "summary"]
                }),
            },
            ScriptTurn::Text {
                text: "已选择 bus_voltage、summary；仍需在研究配置卡中点击“确认并生成脚本”。"
                    .into(),
            },
        ],
    });

    assert_eq!(
        post_and_wait(&app, &session, "bus_voltage，summary").await,
        StatusCode::ACCEPTED
    );
    let trace = factory.trace();
    let history = trace[0].calls[0].history.to_string();
    assert!(history.contains("currentStudyConfiguration"), "{history}");
    assert!(history.contains(&configuration_id), "{history}");
    assert!(
        history.contains(&configuration.revision.to_string()),
        "{history}"
    );
    let events = messages(&app, &session).await;
    assert_eq!(
        tool_names(&events),
        ["load_agent_skill", "update_study_results"]
    );
    let card = events
        .iter()
        .find(|event| {
            event["type"] == "study-config"
                && event["card"]["configurationId"].as_str() == Some(configuration_id.as_str())
        })
        .expect("updated durable study configuration card");
    assert_eq!(card["card"]["revision"], configuration.revision + 1);
    assert_eq!(
        card["card"]["actionCard"]["results"]["selected"],
        json!(["bus_voltage", "summary"])
    );
    assert!(events.iter().any(|event| {
        event["type"] == "assistant"
            && event["content"]
                .as_str()
                .is_some_and(|text| text.contains("仍需") && text.contains("确认并生成脚本"))
    }));
    assert_no_script_or_run(&events);
    assert!(runs(&app, &session).await.is_empty());
    assert!(
        app.state
            .script_service
            .list(&user, session_id)
            .await
            .unwrap()
            .is_empty()
    );
    let mut connection = app.pool.get().unwrap();
    let workflows = sql_query("SELECT count(*) count FROM managed_workflows WHERE session_id = ?1")
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .get_result::<common::CountRow>(&mut connection)
        .unwrap();
    assert_eq!(workflows.count, 0);
}

#[tokio::test]
async fn confirmed_configuration_is_not_injected_as_current_agent_context() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    let session_id = session.parse().unwrap();
    let user = common::test_user("instance");
    import_agent_study_case(&app, &user, session_id).await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    EventWriter::new(
        user.id.clone(),
        session_id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    )
    .study_config(StudyConfigCard {
        configuration_id: configuration.id.to_string(),
        revision: configuration.revision,
        state: "ready_to_confirm".into(),
        action_card: serde_json::to_value(&configuration).unwrap(),
    })
    .await
    .unwrap();
    app.state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "confirm-before-next-agent-turn".into(),
            },
        )
        .await
        .unwrap();
    factory.enqueue(ScriptScenario {
        id: "confirmed-configuration-not-current".into(),
        expected_prompt: Some("再次求解牛顿法潮流".into()),
        turns: vec![ScriptTurn::Text {
            text: "将重新准备研究配置。".into(),
        }],
    });

    assert_eq!(
        post_and_wait(&app, &session, "再次求解牛顿法潮流").await,
        StatusCode::ACCEPTED
    );
    let trace = factory.trace();
    let history = trace[0].calls[0].history.to_string();
    assert!(
        !history.contains("currentStudyConfiguration"),
        "confirmed configuration leaked into agent history: {history}"
    );
}

#[tokio::test]
async fn prepared_configuration_survives_provider_finalization_failure() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
    let session = create_session(&app).await;
    publish_agent_study_model(&app, session.parse().unwrap()).await;
    let explanation = "您希望准备牛顿-拉夫逊潮流研究。请检查当前模型输入、para 参数和母线电压结果，并在确认配置后再开始执行。";
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
                args: json!({"facadeId":"static_power_flow_newton","intent":{"request":"求解牛顿法潮流"},"explanation":explanation}),
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
    let configuration_index = events
        .iter()
        .position(|event| event["type"] == "study-config")
        .expect("study configuration card");
    let acknowledgement_index = events
        .iter()
        .position(|event| {
            event["type"] == "assistant"
                && event["content"].as_str().is_some_and(|text| {
                    text.contains(explanation) && text.contains("已创建“牛顿-拉夫逊潮流”研究配置卡")
                })
        })
        .expect("detailed study explanation");
    assert!(acknowledgement_index < configuration_index);
    assert!(!events.iter().any(|event| {
        event["type"] == "assistant" && event["content"] == "agent 运行失败，请稍后重试"
    }));
}

#[tokio::test]
async fn conversational_agent_cannot_create_a_generic_script() {
    let factory = Arc::new(ScriptedAgentFactory::default());
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
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
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
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
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
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
                args: json!({
                    "facadeId":"motor_dynamics",
                    "intent":{"request":"模拟电机启动暂态"},
                    "explanation":"您希望模拟电机启动暂态。将使用目录中的电机动态门面；请在卡片中补齐动态数据和参数、核对计划结果，并在最终确认后开始执行。"
                }),
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
    let app = TestApp::builder()
        .agent_factory(factory.clone())
        .spawn()
        .await;
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
