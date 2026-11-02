mod common;

use common::TestApp;
use rig::tool::{Tool, ToolContext};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use zjudss::adapters::StubEngine;
use zjudss::application::EventWriter;
use zjudss::application::{
    ModelChange, ModelPatch, PrepareStudyRequest, PublishDraftRequest, StudyConfigurationView,
};
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;
use zjudss::tools::{self, ToolServices};

fn tool_services(app: &TestApp, principal: CurrentUser) -> Arc<ToolServices> {
    Arc::new(ToolServices {
        engine: Arc::new(StubEngine),
        writer: EventWriter::new(
            principal.id.clone(),
            0,
            app.state.message_repo.clone(),
            app.state.events.clone(),
        ),
        session_id: 0,
        request_message_id: 0,
        publication_authorized: false,
        semaphore: Arc::new(Semaphore::new(1)),
        scope: principal.id.clone(),
        principal,
        model_patch: app.state.model_patch_service.clone(),
        model_dataframes: app.state.model_dataframe_read_service.clone(),
        scripts: app.state.script_service.clone(),
        versions: app.state.version_service.clone(),
        runs: app.state.run_service.clone(),
        engine_snapshot: app.state.engine_snapshot.clone(),
        agent_knowledge: app.state.agent_knowledge.clone(),
        loaded_skills: Arc::new(Mutex::new(BTreeSet::new())),
        inspected_workspace_draft: Arc::new(Mutex::new(None)),
        study_configurations: app.state.study_configuration_service.clone(),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
        data_resources: app.state.data_resource_service.clone(),
    })
}

fn tool_services_for_session(
    app: &TestApp,
    principal: CurrentUser,
    session_id: i32,
) -> Arc<ToolServices> {
    let scope = principal.id.clone();
    let mut services = (*tool_services(app, principal)).clone();
    services.session_id = session_id;
    services.writer = EventWriter::new(
        scope,
        session_id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );
    services.loaded_skills = Arc::new(Mutex::new(BTreeSet::from(["model-intake".to_string()])));
    services.publication_authorized = true;
    Arc::new(services)
}

#[test]
fn publication_authorization_requires_an_explicit_positive_current_message() {
    use zjudss::tools::publication_intent_authorized;

    assert!(publication_intent_authorized(
        "请发布当前模型草稿并自动激活"
    ));
    assert!(publication_intent_authorized(
        "发布当前模型版本，然后准备潮流研究"
    ));
    assert!(publication_intent_authorized(
        "Publish the current model draft"
    ));
    assert!(!publication_intent_authorized("通过提示词生成脚本和结果"));
    assert!(!publication_intent_authorized("不要发布，只检查当前草稿"));
    assert!(!publication_intent_authorized("是否可以发布当前模型版本？"));
    assert!(!publication_intent_authorized("请不要自动发布当前模型版本"));
    assert!(!publication_intent_authorized("永远不要发布当前模型版本"));
    assert!(!publication_intent_authorized("告诉我如何发布当前模型版本"));
    assert!(!publication_intent_authorized(
        "Can you publish the current model?"
    ));
    assert!(!publication_intent_authorized("Never publish the model"));
    assert!(!publication_intent_authorized(
        "引用文字：请发布当前模型版本"
    ));
    assert!(!publication_intent_authorized("发布草稿的按钮在哪里"));
    assert!(!publication_intent_authorized(
        "发布当前模型版本需要哪些步骤"
    ));
    assert!(!publication_intent_authorized(
        "发布当前模型版本，然后取消发布"
    ));
    assert!(!publication_intent_authorized("发布当前模型版本，然后算了"));
    assert!(!publication_intent_authorized(
        "发布当前模型草稿并自动激活的按钮在哪里"
    ));
    assert!(!publication_intent_authorized(
        "Publish the current model, then cancel the publication"
    ));
}

async fn apply_csv(
    app: &TestApp,
    session_id: i32,
    client_id: &str,
    file_name: &str,
    bytes: &[u8],
    expected: Option<DraftRef>,
) -> DraftRef {
    let user = common::test_user("alice@example.com");
    common::publish_model_bytes(
        app,
        &user,
        session_id,
        client_id,
        file_name,
        bytes,
        expected,
        file_name.strip_suffix(".csv"),
    )
    .await
}

#[tokio::test]
async fn publish_model_version_requires_model_intake_and_publishes_only_the_exact_draft() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "agent publish", 1)
        .await
        .unwrap();
    let draft = apply_csv(
        &app,
        session.id,
        "agent-publish-source",
        "bus.csv",
        b"id\nb1\n",
        None,
    )
    .await;
    let services = tool_services_for_session(&app, user.clone(), session.id);
    services.loaded_skills.lock().await.clear();
    let maximum_description = "界".repeat(200);
    let args = || tools::publish_model_version::Args {
        expected_draft: draft,
        description: Some(maximum_description.clone()),
    };
    let tool = tools::publish_model_version::PublishModelVersionTool::new(services.clone());

    let missing_skill = tool
        .call(&mut ToolContext::new(), args())
        .await
        .unwrap_err();
    assert_eq!(missing_skill.code, "skill_required");
    services
        .loaded_skills
        .lock()
        .await
        .insert("model-intake".into());

    let missing_inspection = tool
        .call(&mut ToolContext::new(), args())
        .await
        .unwrap_err();
    assert_eq!(missing_inspection.code, "workspace_inspection_required");

    tools::inspect_workspace::InspectWorkspaceTool::new(services.clone())
        .call(&mut ToolContext::new(), tools::inspect_workspace::Args {})
        .await
        .unwrap();

    let invalid_description = tool
        .call(
            &mut ToolContext::new(),
            tools::publish_model_version::Args {
                expected_draft: draft,
                description: Some("界".repeat(201)),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_description.code, "version_description_invalid");

    let published = tool.call(&mut ToolContext::new(), args()).await.unwrap();
    assert_eq!(published["description"], maximum_description);
    assert_eq!(published["active"], true);
    assert_eq!(published["deduplicated"], false);
    assert_eq!(published["replayed"], false);
    assert!(published["versionId"].as_str().is_some());
    assert!(published["contentHash"].as_str().is_some());

    let workspace = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    assert_eq!(
        published["versionId"],
        workspace.active.as_ref().unwrap().version.id.to_string()
    );
    assert!(workspace.draft.is_none());
    let serialized = serde_json::to_string(&published).unwrap();
    assert!(!serialized.contains("artifact"));
    assert!(!serialized.contains("path"));
}

#[tokio::test]
async fn publish_model_version_rejects_a_tool_call_not_authorized_by_the_current_message() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "unauthorized agent publish", 1)
        .await
        .unwrap();
    let draft = apply_csv(
        &app,
        session.id,
        "unauthorized-agent-publish",
        "bus.csv",
        b"id\nb1\n",
        None,
    )
    .await;
    let mut services = (*tool_services_for_session(&app, user, session.id)).clone();
    services.publication_authorized = false;
    let services = Arc::new(services);
    tools::inspect_workspace::InspectWorkspaceTool::new(services.clone())
        .call(&mut ToolContext::new(), tools::inspect_workspace::Args {})
        .await
        .unwrap();

    let error = tools::publish_model_version::PublishModelVersionTool::new(services)
        .call(
            &mut ToolContext::new(),
            tools::publish_model_version::Args {
                expected_draft: draft,
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "publish_authorization_required");

    let workspace = app
        .state
        .version_service
        .workspace(&common::test_user("alice@example.com"), session.id)
        .await
        .unwrap();
    assert!(workspace.active.is_none());
    assert_eq!(workspace.draft.unwrap().draft.id, draft.id);
}

#[tokio::test]
async fn publish_model_version_replays_by_message_and_rejects_stale_or_foreign_drafts() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "agent publish replay", 1)
        .await
        .unwrap();
    let foreign_session = app
        .state
        .session_repo
        .create(&user.id, "foreign draft", 2)
        .await
        .unwrap();
    let first_draft = apply_csv(
        &app,
        session.id,
        "agent-publish-first",
        "bus.csv",
        b"id\nb1\n",
        None,
    )
    .await;
    let mut first_services = (*tool_services_for_session(&app, user.clone(), session.id)).clone();
    first_services.request_message_id = 77;
    let first_tool = tools::publish_model_version::PublishModelVersionTool::new(Arc::new(
        first_services.clone(),
    ));
    tools::inspect_workspace::InspectWorkspaceTool::new(Arc::new(first_services))
        .call(&mut ToolContext::new(), tools::inspect_workspace::Args {})
        .await
        .unwrap();
    let first_args = || tools::publish_model_version::Args {
        expected_draft: first_draft,
        description: None,
    };

    let first = first_tool
        .call(&mut ToolContext::new(), first_args())
        .await
        .unwrap();
    let replay = first_tool
        .call(&mut ToolContext::new(), first_args())
        .await
        .unwrap();
    assert_eq!(replay["versionId"], first["versionId"]);
    assert_eq!(replay["replayed"], true);

    let current_draft = apply_csv(
        &app,
        session.id,
        "agent-publish-current",
        "bus.csv",
        b"id\nb2\n",
        None,
    )
    .await;
    let foreign_draft = apply_csv(
        &app,
        foreign_session.id,
        "agent-publish-foreign",
        "bus.csv",
        b"id\nforeign\n",
        None,
    )
    .await;
    let mut next_services = (*tool_services_for_session(&app, user.clone(), session.id)).clone();
    next_services.request_message_id = 78;
    let next_tool =
        tools::publish_model_version::PublishModelVersionTool::new(Arc::new(next_services.clone()));
    tools::inspect_workspace::InspectWorkspaceTool::new(Arc::new(next_services))
        .call(&mut ToolContext::new(), tools::inspect_workspace::Args {})
        .await
        .unwrap();

    let stale = next_tool
        .call(
            &mut ToolContext::new(),
            tools::publish_model_version::Args {
                expected_draft: first_draft,
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code, "draft_revision_conflict");
    let foreign = next_tool
        .call(
            &mut ToolContext::new(),
            tools::publish_model_version::Args {
                expected_draft: foreign_draft,
                description: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(foreign.code, "draft_revision_conflict");
    let unchanged = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    assert_eq!(unchanged.draft.unwrap().draft.id, current_draft.id);

    let second = next_tool
        .call(
            &mut ToolContext::new(),
            tools::publish_model_version::Args {
                expected_draft: current_draft,
                description: None,
            },
        )
        .await
        .unwrap();
    assert_ne!(second["versionId"], first["versionId"]);
    assert_eq!(second["active"], true);
}

#[tokio::test]
async fn a_new_draft_copies_the_active_version_and_replaces_only_selected_tables() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "draft copy", 1)
        .await
        .unwrap();
    let first = apply_csv(&app, session.id, "bus-first", "bus.csv", b"id\n1\n", None).await;
    let with_line = apply_csv(
        &app,
        session.id,
        "line-first",
        "line.csv",
        b"id\nl1\n",
        Some(first),
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: with_line.id,
                expected_draft_revision: with_line.revision,
                client_publish_id: "publish-two-tables".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(published.version.dataframes.len(), 2);

    let draft = apply_csv(&app, session.id, "bus-change", "bus.csv", b"id\n2\n", None).await;
    let workspace = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    let draft_snapshot = workspace.draft.unwrap();
    assert_eq!(
        draft_snapshot.draft.base_version_id,
        Some(published.version.version.id)
    );
    assert_eq!(draft_snapshot.draft.id, draft.id);
    assert_eq!(draft_snapshot.dataframes.len(), 2);
    assert_eq!(
        draft_snapshot
            .dataframes
            .iter()
            .map(|table| table.table_name.as_str())
            .collect::<Vec<_>>(),
        vec!["bus", "line"]
    );
}

#[tokio::test]
async fn study_tools_search_explicit_functions_and_prepare_persisted_cards() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "study configuration", 1)
        .await
        .unwrap();
    let services = tool_services_for_session(&app, user.clone(), session.id);
    services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let mut context = ToolContext::new();
    let search = tools::search_study_facades::SearchStudyFacadesTool::new(services.clone())
        .call(
            &mut context,
            tools::search_study_facades::Args {
                query: "潮流".into(),
                explicit_function: Some("ds_newton_raphson_pf".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(search["enabled"][0]["id"], "static_power_flow_newton");
    let prepared = tools::prepare_study::PrepareStudyTool::new(services)
        .call(
            &mut context,
            tools::prepare_study::Args {
                facade_id: "static_power_flow_newton".into(),
                intent: serde_json::json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(prepared["facade"]["function"], "ds_newton_raphson_pf");
    assert_eq!(prepared["status"], "collecting_inputs");
    let id = prepared["id"].as_i64().unwrap() as i32;
    assert!(
        app.state
            .study_configuration_service
            .get(&common::test_user("alice@example.com"), id)
            .await
            .is_ok()
    );
    let messages = app
        .state
        .message_repo
        .list_by_session(&user.id, session.id)
        .await
        .unwrap();
    let acknowledgement = messages
        .iter()
        .position(|message| {
            message.role == "assistant"
                && message.content
                    == format!(
                        "已为您创建“牛顿-拉夫逊潮流”研究配置卡（ID: {id}），请在卡片中检查并确认输入、参数和结果。"
                    )
        })
        .expect("deterministic acknowledgement");
    let configuration = messages
        .iter()
        .position(|message| message.kind == "study-config")
        .expect("study configuration message");
    assert!(acknowledgement < configuration);
}

async fn prepare_study_results_configuration(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
) -> StudyConfigurationView {
    app.state
        .study_configuration_service
        .prepare(
            user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: serde_json::json!({"request":"select study results"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn update_study_results_requires_the_study_planning_skill() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "study result skill gate", 1)
        .await
        .unwrap();
    let configuration = prepare_study_results_configuration(&app, &user, session.id).await;
    let services = tool_services_for_session(&app, user.clone(), session.id);
    let mut context = ToolContext::new();

    let error = tools::update_study_results::UpdateStudyResultsTool::new(services)
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into(), "summary".into()],
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "skill_required");
    let persisted = app
        .state
        .study_configuration_service
        .get(&user, configuration.id)
        .await
        .unwrap();
    assert_eq!(persisted.revision, configuration.revision);
    assert_eq!(persisted.results.selected, configuration.results.selected);
}

#[tokio::test]
async fn update_study_results_persists_order_and_revises_the_durable_card() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "study result update", 1)
        .await
        .unwrap();
    let configuration = prepare_study_results_configuration(&app, &user, session.id).await;
    let services = tool_services_for_session(&app, user.clone(), session.id);
    services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let mut context = ToolContext::new();

    let updated = tools::update_study_results::UpdateStudyResultsTool::new(services)
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into(), "summary".into()],
            },
        )
        .await
        .unwrap();

    assert_eq!(updated["revision"], configuration.revision + 1);
    assert_eq!(
        updated["results"]["selected"],
        serde_json::json!(["bus_voltage", "summary"])
    );
    let persisted = app
        .state
        .study_configuration_service
        .get(&user, configuration.id)
        .await
        .unwrap();
    assert_eq!(persisted.revision, configuration.revision + 1);
    assert_eq!(persisted.results.selected, vec!["bus_voltage", "summary"]);

    let messages = app
        .state
        .message_repo
        .list_by_session(&user.id, session.id)
        .await
        .unwrap();
    let card = messages
        .iter()
        .find(|message| {
            message.kind == "study-config"
                && message.operation_id.as_deref() == Some(configuration.id.to_string().as_str())
        })
        .expect("revised study configuration card");
    let payload: serde_json::Value =
        serde_json::from_str(card.payload_json.as_deref().unwrap()).unwrap();
    assert_eq!(payload["revision"], persisted.revision);
    assert_eq!(
        payload["actionCard"]["results"]["selected"],
        serde_json::json!(["bus_voltage", "summary"])
    );
}

#[tokio::test]
async fn update_study_results_rejects_other_sessions_stale_terminal_and_invalid_selections() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "study result validation", 1)
        .await
        .unwrap();
    let other_session = app
        .state
        .session_repo
        .create(&user.id, "other study session", 1)
        .await
        .unwrap();
    let configuration = prepare_study_results_configuration(&app, &user, session.id).await;
    let other_services = tool_services_for_session(&app, user.clone(), other_session.id);
    other_services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let mut context = ToolContext::new();
    let cross_session = tools::update_study_results::UpdateStudyResultsTool::new(other_services)
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into(), "summary".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(cross_session.code, "not_found");

    let services = tool_services_for_session(&app, user.clone(), session.id);
    services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let tool = tools::update_study_results::UpdateStudyResultsTool::new(services);
    let malformed_id = tool
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: "not-an-integer".into(),
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(malformed_id.code, "not_found");
    for results in [
        Vec::<String>::new(),
        vec!["bus_voltage".into(), "bus_voltage".into()],
        vec!["unknown_result".into()],
    ] {
        let error = tool
            .call(
                &mut context,
                tools::update_study_results::Args {
                    configuration_id: configuration.id.to_string(),
                    expected_revision: configuration.revision,
                    results,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "result_selection_invalid");
    }
    let unchanged = app
        .state
        .study_configuration_service
        .get(&user, configuration.id)
        .await
        .unwrap();
    assert_eq!(unchanged.revision, configuration.revision);
    assert_eq!(unchanged.results.selected, configuration.results.selected);

    let revised = tool
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into(), "summary".into()],
            },
        )
        .await
        .unwrap();
    let stale = tool
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: configuration.revision,
                results: vec!["summary".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code, "configuration_stale");

    let cancelled = app
        .state
        .study_configuration_service
        .cancel(
            &user,
            configuration.id,
            revised["revision"].as_i64().unwrap(),
        )
        .await
        .unwrap();
    let terminal = tool
        .call(
            &mut context,
            tools::update_study_results::Args {
                configuration_id: configuration.id.to_string(),
                expected_revision: cancelled.revision,
                results: vec!["bus_voltage".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(terminal.code, "configuration_stale");
}

#[tokio::test]
async fn example_tools_return_manifest_metadata_and_never_physical_paths() {
    let app = TestApp::spawn().await;
    let services = tool_services(&app, common::test_user("alice@example.com"));
    let mut context = ToolContext::new();
    let search = tools::search_examples::SearchExamplesTool::new(services.clone())
        .call(
            &mut context,
            tools::search_examples::Args {
                query: Some("voltage".into()),
                study_type: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(search["snapshotId"], app.state.engine_snapshot.id);
    let examples = search["examples"].as_array().unwrap();
    assert_eq!(examples.len(), 1);
    let summary = &examples[0];
    assert_eq!(summary["title"], "E2E demo");
    assert_eq!(summary["categoryPath"], serde_json::json!(["E2E", "Demo"]));
    assert_eq!(summary["bundlePath"], "E2E/Demo/E2E demo.rssbundle");
    assert_eq!(summary["studyType"], "static_power_flow");
    assert_eq!(summary["solver"], "newton");
    assert_eq!(summary["compatibility"], "opendss-native");
    assert_eq!(summary["results"], serde_json::json!(["voltage"]));
    assert_eq!(summary["sourceEntry"], "dss/model.dss");
    let id = summary["exampleId"].as_str().unwrap();

    let detail = tools::inspect_example::InspectExampleTool::new(services.clone())
        .call(
            &mut context,
            tools::inspect_example::Args {
                example_id: id.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(detail["snapshotId"], app.state.engine_snapshot.id);
    assert_eq!(detail["example"]["manifest"]["schemaVersion"], 2);
    assert_eq!(
        detail["example"]["manifest"]["projection"]["profile"],
        "dss_static_newton_v1"
    );
    assert_eq!(
        detail["example"]["manifest"]["study"]["compatibility"],
        "opendss-native"
    );
    assert_eq!(
        detail["example"]["manifest"]["inputs"],
        serde_json::json!([])
    );
    assert_eq!(
        detail["example"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["manifest", "dss-entry", "main-rss", "managed-rss"]
    );

    let invalid = tools::search_examples::SearchExamplesTool::new(services)
        .call(
            &mut context,
            tools::search_examples::Args {
                query: Some("Broken".into()),
                study_type: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(invalid["examples"][0]["importable"], false);
    assert_eq!(
        invalid["examples"][0]["diagnostics"],
        serde_json::json!(["manifest_invalid"])
    );

    for output in [search, detail, invalid] {
        let serialized = serde_json::to_string(&output).unwrap();
        assert!(!serialized.contains(app.live_engine_root.to_str().unwrap()));
        assert!(!serialized.contains("sourcePath"));
        assert!(!serialized.contains("source_path"));
    }
}

#[tokio::test]
async fn read_model_dataframe_reads_active_and_exact_draft_without_paths() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "read model", 1)
        .await
        .unwrap();
    let initial = apply_csv(
        &app,
        session.id,
        "read-active",
        "bus.csv",
        b"id,enabled,p\nb1,true,10\nb2,false,20\n",
        None,
    )
    .await;
    app.state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: initial.id,
                expected_draft_revision: initial.revision,
                client_publish_id: "publish-read-active".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    let services = tool_services_for_session(&app, user.clone(), session.id);
    let mut context = ToolContext::new();
    let active = tools::read_model_dataframe::ReadModelDataFrameTool::new(services.clone())
        .call(
            &mut context,
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Active,
                table_name: "bus".into(),
                columns: vec!["id".into(), "enabled".into(), "p".into()],
                equals: BTreeMap::new(),
                limit: Some(20),
                expected_draft: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(active["matchCount"], 2);
    assert_eq!(active["actualScope"], "active");
    assert_eq!(active["rows"][0], serde_json::json!(["b1", "true", "10"]));

    let draft = apply_csv(
        &app,
        session.id,
        "read-draft",
        "bus.csv",
        b"id,enabled,p\nb1,true,11\n",
        None,
    )
    .await;
    let draft_result = tools::read_model_dataframe::ReadModelDataFrameTool::new(services)
        .call(
            &mut context,
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                table_name: "bus".into(),
                columns: vec!["id".into(), "p".into()],
                equals: BTreeMap::from([("id".into(), serde_json::json!("b1"))]),
                limit: None,
                expected_draft: Some(draft),
            },
        )
        .await
        .unwrap();
    assert_eq!(draft_result["actualScope"], "draft");
    assert_eq!(draft_result["draftId"], draft.id);
    assert_eq!(draft_result["revision"], draft.revision);
    assert_eq!(draft_result["matchCount"], 1);
    let serialized = serde_json::to_string(&draft_result).unwrap();
    assert!(!serialized.contains("artifactId"));
    assert!(!serialized.contains("storage"));
    assert!(!serialized.contains("path"));
}

#[tokio::test]
async fn read_model_dataframe_reports_exact_match_count_beyond_return_limit() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "exact count", 1)
        .await
        .unwrap();
    let draft = apply_csv(
        &app,
        session.id,
        "exact-count",
        "bus.csv",
        b"id,enabled\na,true\nb,true\nc,true\n",
        None,
    )
    .await;
    let services = tool_services_for_session(&app, user, session.id);
    let result = tools::read_model_dataframe::ReadModelDataFrameTool::new(services)
        .call(
            &mut ToolContext::new(),
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                table_name: "bus".into(),
                columns: vec!["id".into()],
                equals: BTreeMap::from([("enabled".into(), serde_json::json!("true"))]),
                limit: Some(1),
                expected_draft: Some(draft),
            },
        )
        .await
        .unwrap();
    assert_eq!(result["matchCount"], 3);
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn read_model_dataframe_rejects_stale_draft_complex_predicates_and_oversized_output() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "query limits", 1)
        .await
        .unwrap();
    let draft = apply_csv(
        &app,
        session.id,
        "query-limits",
        "bus.csv",
        b"id,p\nb1,10\n",
        None,
    )
    .await;
    let services = tool_services_for_session(&app, user.clone(), session.id);
    let tool = tools::read_model_dataframe::ReadModelDataFrameTool::new(services.clone());
    let stale = tool
        .call(
            &mut ToolContext::new(),
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                table_name: "bus".into(),
                columns: vec!["id".into()],
                equals: BTreeMap::new(),
                limit: None,
                expected_draft: Some(DraftRef {
                    id: draft.id,
                    revision: draft.revision + 1,
                }),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code, "draft_revision_conflict");
    let complex = tool
        .call(
            &mut ToolContext::new(),
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                table_name: "bus".into(),
                columns: vec!["id".into()],
                equals: BTreeMap::from([("id".into(), serde_json::json!(["b1"]))]),
                limit: None,
                expected_draft: Some(draft),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(complex.code, "invalid_model_query");

    let large = format!("id,note\nbig,{}\n", "x".repeat(70 * 1024));
    let large_draft = apply_csv(
        &app,
        session.id,
        "large-output",
        "bus.csv",
        large.as_bytes(),
        Some(draft),
    )
    .await;
    let oversized = tools::read_model_dataframe::ReadModelDataFrameTool::new(services)
        .call(
            &mut ToolContext::new(),
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                table_name: "bus".into(),
                columns: vec!["note".into()],
                equals: BTreeMap::new(),
                limit: Some(1),
                expected_draft: Some(large_draft),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(oversized.code, "model_query_output_too_large");
}

#[tokio::test]
async fn agent_reads_unique_old_value_before_modify_model_advances_only_the_draft() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "read modify", 1)
        .await
        .unwrap();
    let draft = apply_csv(
        &app,
        session.id,
        "modify-source",
        "load.csv",
        b"id,p\nload1,10\n",
        None,
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "publish-modify-source".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    let active_hash = published.version.version.content_hash.clone();
    let services = tool_services_for_session(&app, user.clone(), session.id);
    let read_args = || tools::read_model_dataframe::Args {
        scope: zjudss::application::ModelReadScope::Active,
        table_name: "load".into(),
        columns: vec!["id".into(), "p".into()],
        equals: BTreeMap::from([("id".into(), serde_json::json!("load1"))]),
        limit: Some(2),
        expected_draft: None,
    };
    let read = tools::read_model_dataframe::ReadModelDataFrameTool::new(services.clone())
        .call(&mut ToolContext::new(), read_args())
        .await
        .unwrap();
    assert_eq!(read["matchCount"], 1);
    assert_eq!(read["rows"][0][1], "10");

    let modified = tools::modify_model::ModifyModelTool::new(services.clone())
        .call(
            &mut ToolContext::new(),
            tools::modify_model::Args {
                table_name: "load".into(),
                patch: ModelPatch {
                    row_key: BTreeMap::from([("id".into(), serde_json::json!("load1"))]),
                    changes: BTreeMap::from([(
                        "p".into(),
                        ModelChange {
                            from: serde_json::json!("10"),
                            to: serde_json::json!("20"),
                            unit: "kW".into(),
                        },
                    )]),
                },
                expected_draft: None,
            },
        )
        .await
        .unwrap();
    let current = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    assert_eq!(current.active.unwrap().version.content_hash, active_hash);
    assert_eq!(current.draft.as_ref().unwrap().draft.revision, 1);
    let draft_ref = DraftRef {
        id: modified["draftId"].as_str().unwrap().parse().unwrap(),
        revision: modified["revision"].as_i64().unwrap(),
    };
    let draft_read = tools::read_model_dataframe::ReadModelDataFrameTool::new(services)
        .call(
            &mut ToolContext::new(),
            tools::read_model_dataframe::Args {
                scope: zjudss::application::ModelReadScope::Draft,
                expected_draft: Some(draft_ref),
                ..read_args()
            },
        )
        .await
        .unwrap();
    assert_eq!(draft_read["rows"][0][1], "20");
}
