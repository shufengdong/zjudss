mod common;

use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;

use backend::adapters::StubEngine;
use backend::agent::knowledge::AgentKnowledge;
use backend::application::{ArtifactMetadata, EventWriter};
use backend::auth::CurrentUser;
use backend::storage::StageLimits;
use backend::tools::{self, ToolServices};
use rig::tool::{Tool, ToolContext};
use tokio::sync::{Mutex, Semaphore};

use common::TestApp;

fn principal(key: &str) -> CurrentUser {
    common::test_user(key)
}

fn services(app: &TestApp, principal: CurrentUser) -> Arc<ToolServices> {
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
        task_repo: app.state.task_repo.clone(),
        semaphore: Arc::new(Semaphore::new(1)),
        scope: principal.id.clone(),
        principal,
        workspaces: app.state.workspace_service.clone(),
        model_patch: app.state.model_patch_service.clone(),
        engine_snapshot: app.state.engine_snapshot.clone(),
        agent_knowledge: app.state.agent_knowledge.clone(),
        loaded_skills: Arc::new(Mutex::new(BTreeSet::new())),
        simulations: app.state.simulation_service.clone(),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
    })
}

#[test]
fn manifest_allowed_tools_equal_runtime_registry() {
    let mut manifest = AgentKnowledge::load_embedded("v2.0.1")
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
        21,
        "runtime registry must contain 21 unique tools"
    );
    runtime.dedup();
    assert_eq!(runtime.len(), 21, "runtime tool names must be unique");
    assert_eq!(
        manifest, runtime,
        "manifest and runtime tool registries must be bidirectionally equal"
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
