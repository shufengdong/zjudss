mod common;

use std::io::Cursor;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use zjudss::application::{
    AnalysisInput, AnalysisInputSource, AnalysisRequest, CandidateDecision, CandidatePurpose,
    PublishDraftRequest, UploadFile, UploadSourceKind,
};
use zjudss::auth::CurrentUser;
use zjudss::storage::StageLimits;
use zjudss::testing::{DatasetFixture, EngineScenario, ScenarioEngine};

use common::TestApp;

async fn publish_active_model(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
) -> zjudss::db::repositories::VersionSnapshot {
    let draft = common::publish_model_bytes(
        app,
        user,
        session_id,
        "analysis-api-model",
        "bus.csv",
        b"id,name\n1,source\n",
        None,
        Some("bus"),
    )
    .await;
    app.state
        .version_service
        .publish(
            user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "publish-analysis-api-model".into(),
                description: None,
            },
        )
        .await
        .unwrap()
        .version
}

async fn publish_resource(app: &TestApp, user: &CurrentUser, session_id: i32) -> String {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,value\n1,2\n"), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            "analysis-api-resource".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "analysis-api-resource".into(),
                file_name: "local.csv".into(),
                relative_path: Some("local.csv".into()),
                media_type: Some("text/csv".into()),
                staged,
            }],
        )
        .await
        .unwrap();
    app.state
        .upload_confirmation_service
        .publish_candidates(
            user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: Some("本地量测".into()),
                alias: Some("local_measurement".into()),
            }],
        )
        .await
        .unwrap()
        .resources
        .remove(0)
        .id
}

async fn get_analysis(app: &TestApp, user: &str, id: i32) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/analysis-runs/{id}"))
                .header("cookie", app.login_cookie(user).await)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn analysis_details_return_immutable_script_and_ordered_three_source_snapshots() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Success(vec![DatasetFixture {
        name: "model_result".into(),
        rows: vec![("source".into(), 1.0)],
        value_unit: None,
    }]));
    engine.enqueue(EngineScenario::Success(vec![DatasetFixture {
        name: "combined".into(),
        rows: vec![("source".into(), 2.0)],
        value_unit: None,
    }]));
    let app = TestApp::builder().engine(Arc::new(engine)).spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "analysis details", 1)
        .await
        .unwrap();
    let active = publish_active_model(&app, &user, session.id).await;
    let frame_id = active.dataframes[0].id;
    let model_analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: json!("derive matching dataset"),
            script: "model_result = limit(1, model);".into(),
            inputs: vec![AnalysisInput {
                source: AnalysisInputSource::ActiveModelDataFrameId(frame_id),
                alias: "model".into(),
            }],
            expected_outputs: vec!["model_result".into()],
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let dataset_id = model_analysis.datasets[0].id;
    let resource_id = publish_resource(&app, &user, session.id).await;
    let executed_script = "combined = limit(1, active_bus);";
    let analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user,
            session_id: session.id,
            request_message_id: None,
            intent: json!("combine frozen sources"),
            script: executed_script.into(),
            inputs: vec![
                AnalysisInput {
                    source: AnalysisInputSource::ActiveModelDataFrameId(frame_id),
                    alias: "active_bus".into(),
                },
                AnalysisInput {
                    source: AnalysisInputSource::DatasetId(dataset_id),
                    alias: "prior_result".into(),
                },
                AnalysisInput {
                    source: AnalysisInputSource::DataResourceId(resource_id.clone()),
                    alias: "local_data".into(),
                },
            ],
            expected_outputs: vec!["combined".into()],
            skill_refs: vec![],
        })
        .await
        .unwrap();

    let (status, body) = get_analysis(&app, "alice@example.com", analysis.run.id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["script"]["content"], executed_script);
    assert_eq!(
        body["script"]["contentHash"],
        hex::encode(Sha256::digest(executed_script.as_bytes()))
    );
    assert_eq!(
        body["engineResourceSnapshotId"],
        app.state.engine_snapshot.id
    );
    assert_eq!(body["engineContractVersion"], "tensoreval-parquet-v3");
    let inputs = body["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 3);
    assert_eq!(inputs[0]["ordinal"], 0);
    assert_eq!(inputs[0]["sourceKind"], "active_model");
    assert_eq!(inputs[0]["modelDataframeId"], frame_id.to_string());
    assert_eq!(inputs[0]["modelVersionId"], active.version.id.to_string());
    assert_eq!(inputs[1]["ordinal"], 1);
    assert_eq!(inputs[1]["sourceKind"], "dataset");
    assert_eq!(inputs[1]["datasetId"], dataset_id.to_string());
    assert_eq!(inputs[2]["ordinal"], 2);
    assert_eq!(inputs[2]["sourceKind"], "data_resource");
    assert_eq!(inputs[2]["resourceId"], resource_id);
    for input in inputs {
        assert!(
            input["sourceSchemaHash"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        assert!(
            input["sourceContentHash"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
    }
    let serialized = body.to_string();
    assert!(!serialized.contains(&app.data_dir.display().to_string()));
    assert!(!serialized.contains("storagePath"));
    assert!(!serialized.contains("relativePath"));

    let (foreign_status, foreign_body) =
        get_analysis(&app, "bob@example.com", analysis.run.id).await;
    assert_eq!(foreign_status, StatusCode::NOT_FOUND);
    assert_eq!(foreign_body["code"], "not_found");
}
