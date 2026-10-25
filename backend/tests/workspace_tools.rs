mod common;

use common::TestApp;
use rig::tool::{Tool, ToolContext};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use zjudss::adapters::StubEngine;
use zjudss::application::EventWriter;
use zjudss::application::study_planning::{ResolvedSolver, StudyRequest};
use zjudss::application::{ModelChange, ModelPatch, PublishDraftRequest, UploadBatch, UploadFile};
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;
use zjudss::storage::StageLimits;
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
        study_planning: app.state.study_planning_service.clone(),
        ready_study_plans: Arc::new(Mutex::new(Default::default())),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
    })
}

fn tool_services_for_session(
    app: &TestApp,
    principal: CurrentUser,
    session_id: i32,
) -> Arc<ToolServices> {
    let mut services = (*tool_services(app, principal)).clone();
    services.session_id = session_id;
    services.loaded_skills = Arc::new(Mutex::new(BTreeSet::from(["model-intake".to_string()])));
    Arc::new(services)
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
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session_id,
            client_id.into(),
            expected,
            UploadBatch {
                client_batch_id: client_id.into(),
                files: vec![UploadFile {
                    client_file_id: client_id.into(),
                    file_name: file_name.into(),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let ready = app
        .state
        .model_import_service
        .process(&user, accepted.id.parse().unwrap())
        .await
        .unwrap();
    app.state
        .model_import_service
        .resolve_and_apply(
            &user,
            accepted.id.parse().unwrap(),
            expected,
            ready
                .candidates
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
        )
        .await
        .unwrap()
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
async fn plan_study_issues_ready_ids_only_and_keeps_them_request_local() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let empty_session = app
        .state
        .session_repo
        .create(&user.id, "empty study", 1)
        .await
        .unwrap();
    let empty_services = tool_services_for_session(&app, user.clone(), empty_session.id);
    empty_services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let mut context = ToolContext::new();
    let non_ready = tools::plan_study::PlanStudyTool::new(empty_services)
        .call(
            &mut context,
            tools::plan_study::Args {
                request: StudyRequest::StaticPowerFlow {
                    solver_preference: Default::default(),
                    requested_results: vec![],
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(non_ready["status"], "missing_model_data");
    assert!(non_ready.get("studyPlanId").is_none());

    let session = app
        .state
        .session_repo
        .create(&user.id, "ready study", 1)
        .await
        .unwrap();
    let bus = apply_csv(
        &app,
        session.id,
        "study-bus",
        "bus.csv",
        b"id\nsource\nb1\n",
        None,
    )
    .await;
    let line = apply_csv(
        &app,
        session.id,
        "study-line",
        "line.csv",
        b"name,bus_from,bus_to\nl1,source,b1\n",
        Some(bus),
    )
    .await;
    let source = apply_csv(
        &app,
        session.id,
        "study-source",
        "voltage_source.csv",
        b"name,bus\nsource,source\n",
        Some(line),
    )
    .await;
    app.state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: source.id,
                expected_draft_revision: source.revision,
                client_publish_id: "publish-ready-study".into(),
            },
        )
        .await
        .unwrap();

    let services = tool_services_for_session(&app, user.clone(), session.id);
    services
        .loaded_skills
        .lock()
        .await
        .insert("study-planning".into());
    let ready = tools::plan_study::PlanStudyTool::new(services.clone())
        .call(
            &mut context,
            tools::plan_study::Args {
                request: StudyRequest::StaticPowerFlow {
                    solver_preference: Default::default(),
                    requested_results: vec![],
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["solver"], serde_json::json!(ResolvedSolver::Radial));
    let plan_id = ready["studyPlanId"].as_str().unwrap();
    assert!(plan_id.starts_with("study-plan:"));
    let stored = services.ready_study_plans.lock().await.get(plan_id);
    assert_eq!(stored.unwrap().function, "ds_radial_pf");

    let another_request = tool_services_for_session(&app, user, session.id);
    assert!(
        another_request
            .ready_study_plans
            .lock()
            .await
            .get(plan_id)
            .is_none()
    );
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
    assert_eq!(summary["studyType"], "execution_plan");
    assert_eq!(summary["solver"], "radial");
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
    assert_eq!(detail["example"]["manifest"]["schemaVersion"], 1);
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
        vec!["manifest", "dss-entry", "main-rss"]
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
