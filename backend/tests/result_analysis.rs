mod common;
use backend::application::visualization::{VisualizationBudgets, VisualizationValidator};
use backend::application::{
    AnalysisInput, AnalysisRequest, ArtifactMetadata, DataframeBindingDraft, ModelVersionStatus,
    RunStudyRequest, SourceKind, VersionDraft, VersionInputDraft, VersionOrigin,
};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use backend::storage::StageLimits;
use common::TestApp;
use serde_json::{Value, json};
use std::{collections::HashMap, io::Cursor};
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
async fn source_dataset(app: &TestApp) -> (Principal, i32, i32) {
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.access, "analysis", 1)
        .await
        .unwrap();
    let ws = app
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
        .stage(Cursor::new(b"id,p\nb1,1\n"), StageLimits::default())
        .unwrap();
    let a = app
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
    let v = app
        .state
        .workspace_service
        .publish_version(
            &p.access,
            ws.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({}),
                patch: None,
                inputs: vec![VersionInputDraft {
                    artifact_id: a.id,
                    sha256: a.sha256,
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
    let sim = app
        .state
        .simulation_service
        .run(RunStudyRequest {
            principal: p.clone(),
            session_id: session.id,
            model_version_id: v.id,
            study_type: "power_flow".into(),
            study_spec: json!({}),
            script: Some("result=bus".into()),
            skill_refs: vec![],
        })
        .await
        .unwrap();
    (p, session.id, sim.datasets[0].id)
}

#[tokio::test]
async fn analysis_persists_lineage_script_outputs_views_and_result_message() {
    let app = TestApp::spawn().await;
    let (p, session, dataset) = source_dataset(&app).await;
    let views = HashMap::from([(
        "result".into(),
        vec![("line".into(), json!({"kind":"line","x":"id","y":["value"]}))],
    )]);
    let out = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: p.clone(),
            session_id: session,
            request_message_id: None,
            intent: json!({"question":"show values"}),
            script: "result = source".into(),
            inputs: vec![AnalysisInput {
                dataset_id: dataset,
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: views,
            skill_refs: vec![json!({"id":"result-analysis"})],
        })
        .await
        .unwrap();
    assert_eq!(out.run.status, "succeeded");
    assert_eq!(out.datasets.len(), 1);
    assert_eq!(out.datasets[0].task_id, None);
    assert_eq!(out.datasets[0].analysis_run_id, Some(out.run.id));
    let metadata = app
        .state
        .dataset_service
        .metadata(&p, out.datasets[0].id)
        .await
        .unwrap();
    assert_eq!(metadata.views.len(), 1);
    let data = app
        .state
        .dataset_service
        .view_data(&p, metadata.views[0].id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(data.rows.len(), 1);
    let messages = app
        .state
        .message_repo
        .list_by_session(&p.access, session)
        .await
        .unwrap();
    assert!(messages.iter().any(|m| {
        m.kind == "result"
            && m.payload_json
                .as_deref()
                .unwrap_or("")
                .contains("analysisRunId")
    }));
}

#[test]
fn six_visualizations_validate_and_unsafe_or_oversized_specs_fail_closed() {
    let schema = vec![
        backend::application::datasets::DatasetColumn {
            name: "id".into(),
            dtype: "Utf8".into(),
            nullable: false,
            unit: None,
        },
        backend::application::datasets::DatasetColumn {
            name: "value".into(),
            dtype: "Int64".into(),
            nullable: false,
            unit: Some("kV".into()),
        },
    ];
    let specs: Vec<Value> = vec![
        json!({"kind":"line","x":"id","y":["value"]}),
        json!({"kind":"bar","x":"id","y":["value"]}),
        json!({"kind":"scatter","x":"id","y":["value"]}),
        json!({"kind":"heatmap","x":"id","category":"id","value":"value"}),
        json!({"kind":"boxplot","category":"id","value":"value"}),
        json!({"kind":"histogram","value":"value","bins":20}),
    ];
    for spec in specs {
        VisualizationValidator::parse_and_validate(
            &schema,
            10,
            &spec,
            VisualizationBudgets::default(),
        )
        .unwrap();
    }
    for bad in [
        json!({"kind":"line","x":"missing","y":["value"]}),
        json!({"kind":"line","x":"id","y":["id"]}),
        json!({"kind":"line","x":"id","y":["value"],"formatter":"function(x){}"}),
        json!({"kind":"line","x":"id","y":["value"],"title":"https://evil"}),
    ] {
        assert!(
            VisualizationValidator::parse_and_validate(
                &schema,
                10,
                &bad,
                VisualizationBudgets::default()
            )
            .is_err()
        );
    }
    let too_large = json!({"kind":"scatter","x":"id","y":["value"]});
    let e = VisualizationValidator::parse_and_validate(
        &schema,
        50_001,
        &too_large,
        VisualizationBudgets::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, "visualization_too_large");
}

#[tokio::test]
async fn script_validation_rejects_remote_paths_and_missing_outputs_before_run() {
    let app = TestApp::spawn().await;
    let p = principal();
    for script in ["include \"https://evil/x\"", "read(\"../secret\")"] {
        let e = app
            .state
            .analysis_service
            .run(AnalysisRequest {
                principal: p.clone(),
                session_id: 1,
                request_message_id: None,
                intent: json!({}),
                script: script.into(),
                inputs: vec![AnalysisInput {
                    dataset_id: 1,
                    alias: "source".into(),
                }],
                expected_outputs: vec!["result".into()],
                initial_views: HashMap::new(),
                skill_refs: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "analysis_script_unsafe");
    }
    let e = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: p,
            session_id: 1,
            request_message_id: None,
            intent: json!({}),
            script: "x=source".into(),
            inputs: vec![AnalysisInput {
                dataset_id: 1,
                alias: "source".into(),
            }],
            expected_outputs: vec![],
            initial_views: HashMap::new(),
            skill_refs: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(e.code, "analysis_outputs_required");
}
