mod common;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use backend::adapters::{
    AdapterError, EngineDatasetOutput, EngineRunOutput, ErrorCategory, InspectOutcome, InspectSpec,
    RunSpec, SimEngine,
};
use backend::agent::knowledge::AgentKnowledge;
use backend::application::{
    ArtifactMetadata, DataframeBindingDraft, ModelVersionStatus, RunStudyRequest, SourceKind,
    VersionDraft, VersionInputDraft, VersionOrigin,
};
use backend::application::{SimulationService, StorageLimits};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use backend::db::repositories::SimulationRepo;
use backend::storage::StageLimits;
use common::TestApp;
use serde_json::json;
use std::{io::Cursor, sync::Arc};
use tower::ServiceExt;

fn principal(key: &str) -> Principal {
    Principal {
        access: AccessScope {
            kind: "instance".into(),
            key: key.into(),
        },
        storage: StorageScopeRef {
            kind: "instance".into(),
            key: key.into(),
        },
        subject: key.into(),
    }
}
fn instance() -> Principal {
    principal("instance")
}

async fn ready_dataframe(app: &TestApp, p: &Principal) -> (i32, i32) {
    let session = app
        .state
        .session_repo
        .create(&p.access, "simulation", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&p.access, session.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&p.storage)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,p\nb1,100\n"), StageLimits::default())
        .unwrap();
    let artifact = app
        .state
        .artifact_service
        .commit(
            scope.id,
            staged,
            ArtifactMetadata {
                format: "csv".into(),
                media_type: Some("text/csv".into()),
            },
        )
        .await
        .unwrap();
    let version = app
        .state
        .workspace_service
        .publish_version(
            &p.access,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({"missing":[],"diagnostics":[]}),
                patch: None,
                inputs: vec![VersionInputDraft {
                    artifact_id: artifact.id,
                    sha256: artifact.sha256,
                    role: "input".into(),
                    relative_path: Some("bus.csv".into()),
                    display_name: "bus.csv".into(),
                    source_artifact_id: None,
                    source_artifact_sha256: None,
                    sheet_name: None,
                    binding: Some(DataframeBindingDraft {
                        table_kind: Some("bus".into()),
                        confidence: Some(1.0),
                        resolution: "user_confirmed".into(),
                        columns: json!([{"name":"id","dtype":"string"}]),
                        evidence: json!({}),
                    }),
                }],
            },
        )
        .await
        .unwrap();
    (session.id, version.id)
}

#[tokio::test]
async fn ready_version_persists_reproducible_task_and_generic_parquet_dataset() {
    let app = TestApp::spawn().await;
    let p = instance();
    let (session, version) = ready_dataframe(&app, &p).await;
    let out = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: p.clone(),
            session_id: session,
            model_version_id: version,
            study_type: "power_flow".into(),
            study_spec: json!({"solver":"nr"}),
            script: Some("result = bus".into()),
            skill_refs: vec![
                json!({"id":"study-planning"}),
                json!({"id":"rss-generation"}),
            ],
        })
        .await
        .unwrap();
    assert_eq!(out.task.status, "succeeded");
    assert_eq!(out.task.model_version_id, version);
    assert!(out.task.script_artifact_id > 0);
    assert_eq!(out.task.agent_bundle_version, "v2.0.0");
    assert_eq!(
        out.task.engine_contract_version,
        backend::adapters::ENGINE_CONTRACT_VERSION
    );
    assert_eq!(out.datasets.len(), 1);
    let metadata = app
        .state
        .dataset_service
        .metadata(&p, out.datasets[0].id)
        .await
        .unwrap();
    assert_eq!(metadata.row_count, 1);
    assert_eq!(
        metadata
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "value"]
    );
    let page = app
        .state
        .dataset_service
        .page(&p, out.datasets[0].id, None, None, &[])
        .await
        .unwrap();
    assert_eq!(page.rows, vec![vec![json!("stub"), json!(1)]]);
    assert!(page.next_cursor.is_none());
    let messages = app
        .state
        .message_repo
        .list_by_session(&p.access, session)
        .await
        .unwrap();
    let result = messages.iter().find(|m| m.kind == "result").unwrap();
    assert!(!result.payload_json.as_deref().unwrap().contains("metrics"));
}

#[tokio::test]
async fn rss_studies_execute_the_immutable_entry_artifact_without_generated_script() {
    let app = TestApp::spawn().await;
    let p = instance();
    let session = app
        .state
        .session_repo
        .create(&p.access, "rss entry", 1)
        .await
        .unwrap();
    let snapshot = app.state.example_import_service.snapshot();
    let example = snapshot
        .examples
        .iter()
        .find(|example| example.source_kind == "rss")
        .unwrap();
    let version = app
        .state
        .example_import_service
        .import(&p, session.id, &example.id, &snapshot.id, None)
        .await
        .unwrap();
    let entry = app
        .state
        .workspace_service
        .inputs(&p.access, version.id)
        .await
        .unwrap()
        .into_iter()
        .find(|(input, _)| input.role == "entry")
        .unwrap()
        .0;

    let outcome = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: p,
            session_id: session.id,
            model_version_id: version.id,
            study_type: "power_flow".into(),
            study_spec: json!({}),
            script: None,
            skill_refs: vec![],
        })
        .await
        .unwrap();
    assert_eq!(outcome.task.script_artifact_id, entry.artifact_id);
}

#[tokio::test]
async fn task_dataset_rows_and_download_hide_foreign_scope_and_use_parquet_headers() {
    let app = TestApp::spawn().await;
    let p = instance();
    let (session, version) = ready_dataframe(&app, &p).await;
    let out = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: p.clone(),
            session_id: session,
            model_version_id: version,
            study_type: "short_circuit".into(),
            study_spec: json!({}),
            script: Some("result = bus".into()),
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let dataset = out.datasets[0].id;
    let foreign = principal("foreign");
    assert!(matches!(
        app.state.dataset_service.metadata(&foreign, dataset).await,
        Err(backend::application::DatasetError::NotFound)
    ));
    for uri in [
        format!("/api/tasks/{}", out.task.id),
        format!("/api/tasks/{}/datasets", out.task.id),
        format!("/api/datasets/{dataset}"),
        format!("/api/datasets/{dataset}/rows"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/datasets/{dataset}/download"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/vnd.apache.parquet"
    );
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains(&format!("{dataset}-result.parquet"))
    );
}

#[derive(Clone, Copy, Debug)]
enum FailureMode {
    Engine,
    BadParquet,
    Duplicate,
}
#[derive(Clone)]
struct FailureEngine(FailureMode);
#[async_trait]
impl SimEngine for FailureEngine {
    async fn inspect(&self, _: InspectSpec) -> Result<InspectOutcome, AdapterError> {
        Ok(InspectOutcome {
            ok: true,
            summary: "ok".into(),
            diagnostics: None,
        })
    }
    async fn run_dataframe(&self, s: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        match self.0 {
            FailureMode::Engine => Err(AdapterError::new(
                "solver_failed",
                ErrorCategory::Solver,
                "fixture failure",
            )),
            FailureMode::BadParquet => {
                std::fs::write(s.result_dir.join("bad.parquet"), b"bad").unwrap();
                Ok(EngineRunOutput {
                    datasets: vec![EngineDatasetOutput {
                        name: "bad".into(),
                        path: s.result_dir.join("bad.parquet"),
                        metadata: backend::storage::ParquetMetadata {
                            columns: vec![],
                            row_count: 0,
                            size_bytes: 3,
                            snappy: false,
                        },
                    }],
                })
            }
            FailureMode::Duplicate => {
                let stub = backend::adapters::StubEngine;
                let first = stub.run_dataframe(s.clone()).await?;
                let source = &first.datasets[0].path;
                let second = s.result_dir.join("result-2.parquet");
                std::fs::copy(source, &second).unwrap();
                let mut datasets = first.datasets;
                datasets.push(EngineDatasetOutput {
                    name: "result".into(),
                    path: second.clone(),
                    metadata: backend::storage::ParquetInspector::metadata(&second).unwrap(),
                });
                Ok(EngineRunOutput { datasets })
            }
        }
    }
    async fn run_bundle(&self, s: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.run_dataframe(s).await
    }
    async fn run_analysis(&self, s: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        self.run_dataframe(s).await
    }
}
fn failure_service(app: &TestApp, mode: FailureMode) -> SimulationService {
    SimulationService::new(
        SimulationRepo::new(app.pool.clone()),
        app.state.workspace_service.clone(),
        app.state.artifact_service.clone(),
        app.state.message_repo.clone(),
        app.state.events.clone(),
        Arc::new(FailureEngine(mode)),
        Arc::new(AgentKnowledge::load_embedded("v2.0.0").unwrap()),
        app.state.data_dir.clone(),
    )
}

#[tokio::test]
async fn engine_bad_parquet_quota_and_db_failure_publish_no_partial_result() {
    for mode in [
        FailureMode::Engine,
        FailureMode::BadParquet,
        FailureMode::Duplicate,
    ] {
        let app = TestApp::spawn().await;
        let p = instance();
        let (session, version) = ready_dataframe(&app, &p).await;
        let error = failure_service(&app, mode)
            .run(RunStudyRequest {
                principal: p.clone(),
                session_id: session,
                model_version_id: version,
                study_type: "power_flow".into(),
                study_spec: json!({}),
                script: Some(format!("script-{mode:?}")),
                skill_refs: vec![],
            })
            .await
            .unwrap_err();
        let id = error.task_id.unwrap();
        let task = SimulationRepo::new(app.pool.clone())
            .find(&p.access, id)
            .await
            .unwrap();
        assert_eq!(task.status, "failed");
        assert!(
            app.state
                .dataset_service
                .list(&p, id)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !app.state
                .data_dir
                .join("task-work")
                .join(id.to_string())
                .exists()
        );
        assert!(
            app.state
                .message_repo
                .list_by_session(&p.access, session)
                .await
                .unwrap()
                .iter()
                .all(|m| m.kind != "result")
        );
    }
    let app = TestApp::spawn().await;
    let p = instance();
    let (session, version) = ready_dataframe(&app, &p).await;
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&p.storage)
        .await
        .unwrap();
    let usage = app
        .state
        .artifact_service
        .get_usage(scope.id)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            scope.id,
            StorageLimits {
                artifact_limit: 100,
                byte_limit: usage.used_bytes + 12,
                warning_ratio: 0.8,
            },
        )
        .await
        .unwrap();
    let error = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: p.clone(),
            session_id: session,
            model_version_id: version,
            study_type: "power_flow".into(),
            study_spec: json!({}),
            script: Some("small-script".into()),
            skill_refs: vec![],
        })
        .await
        .unwrap_err();
    let id = error.task_id.unwrap();
    assert_eq!(
        SimulationRepo::new(app.pool.clone())
            .find(&p.access, id)
            .await
            .unwrap()
            .status,
        "failed"
    );
    assert!(
        app.state
            .dataset_service
            .list(&p, id)
            .await
            .unwrap()
            .is_empty()
    );
}
