mod common;

use backend::adapters::StubEngine;
use backend::application::EventWriter;
use backend::application::{PublishDraftRequest, UploadBatch, UploadFile};
use backend::auth::CurrentUser;
use backend::db::models::DraftRef;
use backend::storage::StageLimits;
use backend::tools::{self, ToolServices};
use common::TestApp;
use rig::tool::{Tool, ToolContext};
use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

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
        scripts: app.state.script_service.clone(),
        versions: app.state.version_service.clone(),
        runs: app.state.run_service.clone(),
        engine_snapshot: app.state.engine_snapshot.clone(),
        agent_knowledge: app.state.agent_knowledge.clone(),
        loaded_skills: Arc::new(Mutex::new(BTreeSet::new())),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
    })
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
