mod common;

use std::io::Cursor;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{
    IngestionError, ModelVersionStatus, SourceKind, StorageLimits, UploadBatch, UploadFile,
    VersionDraft, VersionOrigin,
};
use backend::auth::{CurrentUser, UserId};
use backend::storage::StageLimits;
use serde_json::json;
use tower::ServiceExt;

use common::TestApp;

fn principal() -> CurrentUser {
    CurrentUser {
        id: UserId("instance".into()),
        name: "Instance".into(),
        storage_scope_id: 1,
        auth_session_hash: "test".into(),
    }
}

fn batch(app: &TestApp, batch_id: &str, files: &[(&str, &str, &[u8])]) -> UploadBatch {
    UploadBatch {
        client_batch_id: batch_id.to_string(),
        files: files
            .iter()
            .map(|(id, name, bytes)| UploadFile {
                client_file_id: (*id).to_string(),
                file_name: (*name).to_string(),
                relative_path: None,
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(bytes.to_vec()), StageLimits::default())
                    .unwrap(),
            })
            .collect(),
    }
}

async fn session(app: &TestApp) -> i32 {
    app.state
        .session_repo
        .create(&principal().id, "ingestion", 1)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn batch_is_idempotent_by_file_id_and_hash_and_allows_duplicate_names() {
    let app = TestApp::spawn().await;
    let session_id = session(&app).await;
    let first = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(
                &app,
                "batch-1",
                &[("a", "same.csv", b"id\n1\n"), ("b", "same.csv", b"id\n2\n")],
            ),
        )
        .await
        .unwrap();
    let retry = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(
                &app,
                "batch-1",
                &[
                    ("a", "renamed.csv", b"id\n1\n"),
                    ("b", "same.csv", b"id\n2\n"),
                ],
            ),
        )
        .await
        .unwrap();
    assert_eq!(first.id, retry.id);
    assert_eq!(first.items.len(), 2);
    assert!(first.items.iter().all(|item| item.role == "uploaded"));

    let conflict = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(
                &app,
                "batch-1",
                &[
                    ("a", "same.csv", b"id\nchanged\n"),
                    ("b", "same.csv", b"id\n2\n"),
                ],
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(conflict, IngestionError::IdempotencyConflict));
}

#[tokio::test]
async fn partial_failure_publishes_collecting_draft_and_persists_recoverable_state() {
    let app = TestApp::spawn().await;
    let session_id = session(&app).await;
    let accepted = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(
                &app,
                "batch-partial",
                &[
                    ("good", "bus.csv", b"id\nb1\n"),
                    ("bad", "notes.txt", b"not a table"),
                ],
            ),
        )
        .await
        .unwrap();
    app.state
        .ingestion_service
        .process(principal(), accepted.id.parse().unwrap())
        .await;

    let recovered = app
        .state
        .ingestion_service
        .get(&principal(), accepted.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(recovered.status, "completed_with_errors");
    assert!(recovered.published_version_id.is_some());
    assert_eq!(recovered.stages.len(), 4);
    assert!(recovered.items.iter().any(|item| item.status == "ready"));
    assert!(recovered.items.iter().any(|item| item.status == "failed"));
    let workspace = app
        .state
        .workspace_service
        .get(&principal().id, session_id)
        .await
        .unwrap();
    assert_eq!(workspace.draft.as_ref().unwrap().status, "collecting");
}

#[tokio::test]
async fn quota_failure_does_not_advance_workspace_and_stale_base_is_recorded() {
    let app = TestApp::spawn().await;
    let session_id = session(&app).await;
    let scope = app
        .state
        .artifact_service
        .resolve_scope(principal().storage_scope_id)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            scope.id,
            StorageLimits {
                artifact_limit: 1,
                byte_limit: 2,
                warning_ratio: 0.8,
            },
        )
        .await
        .unwrap();
    let quota = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(&app, "too-large", &[("a", "a.csv", b"id\n1\n")]),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        quota,
        IngestionError::Artifact(backend::application::ArtifactError::QuotaExceeded)
    ));
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&principal().id, session_id)
        .await
        .unwrap();
    assert!(workspace.workspace.draft_version_id.is_none());

    app.state
        .artifact_service
        .set_limits(
            scope.id,
            StorageLimits {
                artifact_limit: 100,
                byte_limit: 1024 * 1024,
                warning_ratio: 0.8,
            },
        )
        .await
        .unwrap();
    let accepted = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(&app, "stale", &[("a", "a.csv", b"id\n1\n")]),
        )
        .await
        .unwrap();
    app.state
        .workspace_service
        .publish_version(
            &principal().id,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Generated,
                manifest: json!({}),
                patch: None,
                inputs: vec![],
            },
        )
        .await
        .unwrap();
    app.state
        .ingestion_service
        .process(principal(), accepted.id.parse().unwrap())
        .await;
    let stale = app
        .state
        .ingestion_service
        .get(&principal(), accepted.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(stale.status, "failed");
    assert_eq!(stale.error_code.as_deref(), Some("version_conflict"));
}

#[tokio::test]
async fn multipart_api_returns_202_and_get_restores_terminal_state() {
    let app = TestApp::spawn().await;
    let session_id = session(&app).await;
    let cookie = app.login_cookie("instance").await;
    let boundary = "zjudss-v2-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"clientBatchId\"\r\n\r\nhttp-batch\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file.bus-file\"; filename=\"bus.csv\"\r\nContent-Type: text/csv\r\n\r\nid\nb1\n\r\n\
         --{boundary}--\r\n"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session_id}/attachments"))
                .header("cookie", &cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let ingestion_id = response["ingestionId"].as_str().unwrap();
    assert_eq!(response["accepted"][0]["clientFileId"], "bus-file");

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(
                    Request::get(format!("/api/ingestions/{ingestion_id}"))
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            if matches!(
                value["ingestion"]["status"].as_str(),
                Some("succeeded" | "completed_with_errors" | "failed")
            ) {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background ingestion reaches a persisted terminal state");
    assert_eq!(terminal["ingestion"]["status"], "succeeded");
    assert!(terminal["ingestion"]["publishedVersionId"].is_string());
}

#[tokio::test]
async fn excel_nonempty_sheets_are_normalized_to_traceable_parquet_artifacts() {
    let app = TestApp::spawn().await;
    let session_id = session(&app).await;
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/inference/multi.xlsx"),
    )
    .unwrap();
    let accepted = app
        .state
        .ingestion_service
        .accept_batch(
            &principal(),
            session_id,
            batch(&app, "excel", &[("book", "model.xlsx", &bytes)]),
        )
        .await
        .unwrap();
    app.state
        .ingestion_service
        .process(principal(), accepted.id.parse().unwrap())
        .await;
    let recovered = app
        .state
        .ingestion_service
        .get(&principal(), accepted.id.parse().unwrap())
        .await
        .unwrap();
    let normalized: Vec<_> = recovered
        .items
        .iter()
        .filter(|item| item.role == "normalized")
        .collect();
    assert_eq!(normalized.len(), 1, "empty sheets must be ignored");
    assert_eq!(normalized[0].sheet_name.as_deref(), Some("Loads"));
    assert!(normalized[0].source_artifact_id.is_some());
    assert_eq!(
        normalized[0].media_type.as_deref(),
        Some("application/vnd.apache.parquet")
    );
}
