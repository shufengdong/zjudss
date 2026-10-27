mod common;

use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;

use rig::tool::{Tool, ToolContext};
use tokio::sync::{Mutex, Semaphore};
use zjudss::adapters::StubEngine;
use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::application::{
    AnalysisInput, AnalysisInputSource, AnalysisRequest, ArtifactMetadata, CandidateDecision,
    CandidatePurpose, EventWriter, UploadFile, UploadSourceKind,
};
use zjudss::auth::CurrentUser;
use zjudss::storage::StageLimits;
use zjudss::tools::{self, ToolServices};

use common::TestApp;

fn principal(key: &str) -> CurrentUser {
    common::test_user(key)
}

fn services(app: &TestApp, principal: CurrentUser) -> Arc<ToolServices> {
    services_for_session(app, principal, 0)
}

fn services_for_session(
    app: &TestApp,
    principal: CurrentUser,
    session_id: i32,
) -> Arc<ToolServices> {
    Arc::new(ToolServices {
        engine: Arc::new(StubEngine),
        writer: EventWriter::new(
            principal.id.clone(),
            session_id,
            app.state.message_repo.clone(),
            app.state.events.clone(),
        ),
        session_id,
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
        study_configurations: app.state.study_configuration_service.clone(),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
        data_resources: app.state.data_resource_service.clone(),
    })
}

#[tokio::test]
async fn data_resource_tools_list_read_rename_and_promote_owned_session_resources() {
    let app = TestApp::spawn().await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "resource tools", 1)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"x,value\n1,2\n"), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session.id,
            "tool-resource".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "tool-resource".into(),
                file_name: "tool-resource.csv".into(),
                relative_path: Some("tool-resource.csv".into()),
                media_type: Some("text/csv".into()),
                staged,
            }],
        )
        .await
        .unwrap();
    let published = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: Some("工具数据".into()),
                alias: Some("tool_data".into()),
            }],
        )
        .await
        .unwrap();
    let resource_id = published.resources[0].id.clone();
    let tool_services = services_for_session(&app, user.clone(), session.id);
    let mut context = ToolContext::new();

    let listed = tools::list_data_resources::ListDataResourcesTool::new(tool_services.clone())
        .call(
            &mut context,
            tools::list_data_resources::Args {
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();
    assert_eq!(listed["resources"][0]["id"], resource_id);
    let read = tools::read_data_resource::ReadDataResourceTool::new(tool_services.clone())
        .call(
            &mut context,
            tools::read_data_resource::Args {
                resource_id: resource_id.clone(),
                columns: vec!["value".into()],
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(read["page"]["rows"][0][0], "2");
    let renamed =
        tools::update_data_resource_alias::UpdateDataResourceAliasTool::new(tool_services.clone())
            .call(
                &mut context,
                tools::update_data_resource_alias::Args {
                    resource_id: resource_id.clone(),
                    alias: "renamed_tool_data".into(),
                },
            )
            .await
            .unwrap();
    assert_eq!(renamed["alias"], "renamed_tool_data");

    let analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: serde_json::json!({"goal":"copy"}),
            script: "result = source".into(),
            inputs: vec![AnalysisInput {
                source: AnalysisInputSource::DataResourceId(resource_id),
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: Default::default(),
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let promoted = tools::promote_result_dataset::PromoteResultDatasetTool::new(tool_services)
        .call(
            &mut context,
            tools::promote_result_dataset::Args {
                dataset_id: analysis.datasets[0].id,
                display_name: "提升结果".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(promoted["sourceKind"], "analysis");
    assert_eq!(promoted["lineage"]["datasetId"], analysis.datasets[0].id);
}

#[test]
fn manifest_allowed_tools_equal_runtime_registry() {
    let mut manifest = AgentKnowledge::load_embedded("v4.0.0")
        .unwrap()
        .manifest
        .allowed_tools;
    let mut runtime = tools::registered_tool_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    manifest.sort();
    runtime.sort();

    assert_eq!(
        runtime.len(),
        22,
        "runtime registry must contain 22 unique tools"
    );
    runtime.dedup();
    assert_eq!(runtime.len(), 22, "runtime tool names must be unique");
    assert!(!runtime.iter().any(|tool| tool == "create_script"));
    assert_eq!(
        manifest, runtime,
        "manifest and runtime tool registries must be bidirectionally equal"
    );
}

#[tokio::test]
async fn registered_tools_preserve_domain_error_feedback_and_metadata() {
    let app = TestApp::spawn().await;
    let registered = tools::build_tools(services(&app, principal("alice")));
    let planner = registered
        .iter()
        .find(|tool| tool.name() == "search_study_facades")
        .expect("search_study_facades registered");
    let result = planner
        .execute(
            r#"{"query":"潮流","explicitFunction":null}"#.into(),
            &mut ToolContext::new(),
        )
        .await;
    let error = result.error().expect("missing skill should fail");

    assert_eq!(error.code(), Some("skill_required"));
    assert_eq!(error.retryable(), Some(false));
    assert_ne!(error.model_feedback(), Some("the tool failed"));
    assert!(
        error
            .model_feedback()
            .is_some_and(|feedback| feedback.contains("skill_required"))
    );
}

#[tokio::test]
async fn get_storage_usage_returns_current_scope_only() {
    let app = TestApp::spawn().await;
    let artifacts = &app.state.artifact_service;
    for (scope_id, bytes) in [(2, b"alice".as_slice()), (3, b"bob".as_slice())] {
        let scope = artifacts.resolve_scope(scope_id).await.unwrap();
        let staged = artifacts
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .unwrap();
        artifacts
            .commit(
                scope.id,
                staged,
                ArtifactMetadata {
                    format: "bin".into(),
                    media_type: Some("application/octet-stream".into()),
                },
            )
            .await
            .unwrap();
    }

    let mut context = ToolContext::new();
    let value =
        tools::get_storage_usage::GetStorageUsageTool::new(services(&app, principal("alice")))
            .call(&mut context, tools::get_storage_usage::Args {})
            .await
            .unwrap();

    assert_eq!(value["scope"], "user");
    assert_eq!(value["artifactCount"], 1);
    assert_eq!(value["logicalBytes"], 5);
    assert_eq!(value["artifactLimit"], 20_000);
    assert_eq!(value["byteLimit"], 20_i64 * 1024 * 1024 * 1024);
    assert_eq!(value["warningRatio"], 0.8);
    assert_eq!(value["isWarning"], false);
    assert_eq!(value["isExceeded"], false);
}
