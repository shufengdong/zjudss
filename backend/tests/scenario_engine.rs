#![cfg(feature = "e2e-test")]

mod common;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use backend::adapters::{OutputLimits, RunMode, RunSpec, SimEngine};
use backend::application::{
    AnalysisInput, AnalysisRequest, ArtifactMetadata, DataframeBindingDraft, ModelVersionStatus,
    RunStudyRequest, SourceKind, VersionDraft, VersionInputDraft, VersionOrigin,
};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use backend::db::repositories::SimulationRepo;
use backend::storage::StageLimits;
use backend::testing::{DatasetFixture, EngineScenario, ScenarioEngine};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::json;
use sha2::{Digest, Sha256};

use common::TestApp;

fn run_spec(root: &std::path::Path) -> RunSpec {
    let input = root.join("input");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("study.rss"), "1").unwrap();
    RunSpec {
        mode: RunMode::Dataframe,
        input_tree: input,
        script_path: PathBuf::from("study.rss"),
        named_inputs: vec![],
        result_dir: root.join("results"),
        timeout: Duration::from_secs(1),
    }
}

fn fixture(name: &str, rows: &[(&str, f64)], unit: &str) -> DatasetFixture {
    DatasetFixture {
        name: name.into(),
        rows: rows
            .iter()
            .map(|(key, value)| ((*key).to_string(), *value))
            .collect(),
        value_unit: Some(unit.into()),
    }
}

fn sha256(path: &std::path::Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

fn assert_manifest_dataset(
    expected: &serde_json::Value,
    actual: &backend::adapters::EngineDatasetOutput,
) {
    assert_eq!(expected["name"], actual.name);
    assert_eq!(expected["rowCount"], actual.metadata.row_count);
    assert_eq!(expected["sha256"], sha256(&actual.path));
    let reader =
        ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&actual.path).unwrap())
            .unwrap();
    let schema = reader.schema();
    for (index, column) in expected["columns"].as_array().unwrap().iter().enumerate() {
        assert_eq!(column["name"], actual.metadata.columns[index].name);
        assert_eq!(column["dtype"], actual.metadata.columns[index].dtype);
        assert_eq!(column["nullable"], actual.metadata.columns[index].nullable);
        if let Some(unit) = column.get("unit") {
            assert_eq!(
                schema
                    .field(index)
                    .metadata()
                    .get("unit")
                    .map(String::as_str),
                unit.as_str()
            );
        }
    }
}

fn principal() -> Principal {
    Principal {
        access: AccessScope {
            kind: "instance".into(),
            key: "instance".into(),
        },
        storage: StorageScopeRef {
            kind: "instance".into(),
            key: "instance".into(),
        },
        subject: "instance".into(),
    }
}

async fn ready_dataframe(app: &TestApp, principal: &Principal) -> (i32, i32) {
    let session = app
        .state
        .session_repo
        .create(&principal.access, "scenario", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&principal.access, session.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&principal.storage)
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
            &principal.access,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({"missing": []}),
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
                        columns: json!([]),
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
async fn success_scenarios_write_snappy_parquet_matching_fixture_manifests() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Success(vec![fixture(
        "voltage",
        &[("bus-1", 1.0), ("bus-2", 0.98)],
        "pu",
    )]));
    engine.enqueue(EngineScenario::Success(vec![
        fixture("voltage", &[("bus-1", 1.0)], "pu"),
        fixture("current", &[("line-1", 12.5)], "A"),
    ]));

    let single_root = tempfile::tempdir().unwrap();
    let single = engine
        .run_dataframe(run_spec(single_root.path()))
        .await
        .unwrap();
    assert_eq!(single.datasets.len(), 1);
    assert_eq!(single.datasets[0].metadata.row_count, 2);
    assert!(single.datasets[0].metadata.snappy);
    assert_eq!(single.datasets[0].metadata.columns[1].name, "value");

    let multi_root = tempfile::tempdir().unwrap();
    let multi = engine
        .run_dataframe(run_spec(multi_root.path()))
        .await
        .unwrap();
    assert_eq!(multi.datasets.len(), 2);
    assert!(multi.datasets.iter().all(|dataset| dataset.metadata.snappy));

    let single_manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../integration-tests/fixtures/results/single/manifest.json"
    ))
    .unwrap();
    assert_eq!(single_manifest["generator"], "scenario-engine-v1");
    assert_manifest_dataset(&single_manifest["datasets"][0], &single.datasets[0]);
    let multi_manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../integration-tests/fixtures/results/multi/manifest.json"
    ))
    .unwrap();
    for dataset in &multi.datasets {
        let expected = multi_manifest["datasets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == dataset.name)
            .unwrap();
        assert_manifest_dataset(expected, dataset);
    }
    assert_eq!(engine.calls().len(), 2);
}

#[tokio::test]
async fn failure_scenarios_are_rejected_by_the_adapter_contract() {
    let limits = OutputLimits {
        max_files: 4,
        max_file_bytes: 1024,
        max_total_bytes: 2048,
    };
    let cases = [
        (EngineScenario::RunFailure, "scenario_run_failed"),
        (EngineScenario::Empty, "result_empty"),
        (EngineScenario::CorruptParquet, "parquet_footer_invalid"),
        (EngineScenario::Oversize, "result_file_too_large"),
        (EngineScenario::EscapingPath, "result_file_invalid"),
        (EngineScenario::Symlink, "result_symlink_rejected"),
    ];
    let inspect_engine = ScenarioEngine::with_output_limits(limits);
    inspect_engine.enqueue(EngineScenario::InspectFailure);
    let inspect_root = tempfile::tempdir().unwrap();
    let inspect_spec = run_spec(inspect_root.path());
    let inspect_error = inspect_engine
        .inspect(backend::adapters::InspectSpec {
            input_tree: inspect_spec.input_tree,
            model_path: inspect_spec.script_path,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert_eq!(inspect_error.code, "scenario_inspect_failed");
    for (scenario, code) in cases {
        let engine = ScenarioEngine::with_output_limits(limits);
        engine.enqueue(scenario);
        let root = tempfile::tempdir().unwrap();
        let error = engine
            .run_dataframe(run_spec(root.path()))
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(engine.calls()[0].result_dir.starts_with(root.path()));
    }
}

#[tokio::test]
async fn timeout_barrier_releases_without_sleep() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Timeout {
        barrier_id: "engine-timeout".into(),
    });
    let root = tempfile::tempdir().unwrap();
    let spec = run_spec(root.path());
    let running = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.run_dataframe(spec).await })
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        engine.wait_until_blocked("engine-timeout"),
    )
    .await
    .expect("timeout barrier reached");
    assert!(engine.release("engine-timeout"));
    let error = running.await.unwrap().unwrap_err();
    assert_eq!(error.code, "engine_timeout");
}

#[tokio::test]
async fn simulation_and_analysis_fail_without_partial_publication_and_clean_staging() {
    let limits = OutputLimits {
        max_files: 4,
        max_file_bytes: 1024,
        max_total_bytes: 2048,
    };
    let engine = Arc::new(ScenarioEngine::with_output_limits(limits));
    engine.enqueue(EngineScenario::CorruptParquet);
    let app = TestApp::spawn_with_engine(engine.clone()).await;
    let principal = principal();
    let (session_id, version_id) = ready_dataframe(&app, &principal).await;
    let simulation_error = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: principal.clone(),
            session_id,
            model_version_id: version_id,
            study_type: "power_flow".into(),
            study_spec: json!({}),
            script: Some("result = bus".into()),
            skill_refs: vec![],
        })
        .await
        .unwrap_err();
    let task_id = simulation_error.task_id.unwrap();
    let task = SimulationRepo::new(app.pool.clone())
        .find(&principal.access, task_id)
        .await
        .unwrap();
    assert_eq!(task.status, "failed");
    assert!(
        app.state
            .dataset_service
            .list(&principal, task_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !app.state
            .data_dir
            .join("task-work")
            .join(task_id.to_string())
            .exists()
    );

    engine.enqueue(EngineScenario::Success(vec![fixture(
        "source",
        &[("bus-1", 1.0)],
        "pu",
    )]));
    let successful = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: principal.clone(),
            session_id,
            model_version_id: version_id,
            study_type: "power_flow".into(),
            study_spec: json!({}),
            script: Some("source = bus".into()),
            skill_refs: vec![],
        })
        .await
        .unwrap();
    engine.enqueue(EngineScenario::RunFailure);
    let analysis_error = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: principal.clone(),
            session_id,
            request_message_id: None,
            intent: json!({"question": "summarize"}),
            script: "result = source".into(),
            inputs: vec![AnalysisInput {
                dataset_id: successful.datasets[0].id,
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: HashMap::new(),
            skill_refs: vec![],
        })
        .await
        .unwrap_err();
    let analysis_id = analysis_error.run_id.unwrap();
    let (run, datasets) = app
        .state
        .analysis_service
        .find(&principal, analysis_id)
        .await
        .unwrap();
    assert_eq!(run.status, "failed");
    assert!(datasets.is_empty());
    assert!(
        !app.state
            .data_dir
            .join("analysis-work")
            .join(analysis_id.to_string())
            .exists()
    );
    assert_eq!(
        std::fs::read_dir(app.state.data_dir.join("staging"))
            .unwrap()
            .count(),
        0
    );
}
