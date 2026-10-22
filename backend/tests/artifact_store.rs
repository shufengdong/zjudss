mod common;

use std::io::Cursor;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{ArtifactError, ArtifactMetadata, StorageLimits};
use backend::auth::StorageScopeRef;
use backend::storage::StageLimits;
use tower::ServiceExt;

use common::TestApp;

fn owner(key: &str) -> StorageScopeRef {
    StorageScopeRef {
        kind: "owner".to_string(),
        key: key.to_string(),
    }
}

fn metadata(_name: &str) -> ArtifactMetadata {
    ArtifactMetadata {
        format: "bin".to_string(),
        media_type: Some("application/octet-stream".to_string()),
    }
}

#[tokio::test]
async fn deduplicates_per_scope_but_charges_each_scope() {
    let app = TestApp::spawn().await;
    let service = &app.state.artifact_service;
    let alice = service
        .resolve_scope(&owner("alice"))
        .await
        .expect("alice scope");
    let bob = service
        .resolve_scope(&owner("bob"))
        .await
        .expect("bob scope");

    let first = service
        .commit(
            alice.id,
            service
                .store()
                .stage(Cursor::new(b"same bytes"), StageLimits::default())
                .expect("stage first"),
            metadata("first.csv"),
        )
        .await
        .expect("commit first");
    let repeated = service
        .commit(
            alice.id,
            service
                .store()
                .stage(Cursor::new(b"same bytes"), StageLimits::default())
                .expect("stage repeated"),
            metadata("repeated.csv"),
        )
        .await
        .expect("commit repeated");
    let cross_scope = service
        .commit(
            bob.id,
            service
                .store()
                .stage(Cursor::new(b"same bytes"), StageLimits::default())
                .expect("stage bob"),
            metadata("bob.csv"),
        )
        .await
        .expect("commit bob");

    assert_eq!(first.id, repeated.id);
    assert_eq!(first.id, cross_scope.id);
    assert_eq!(service.get_usage(alice.id).await.unwrap().used_artifacts, 1);
    assert_eq!(service.get_usage(bob.id).await.unwrap().used_artifacts, 1);
    assert_eq!(service.get_usage(alice.id).await.unwrap().used_bytes, 10);
}

#[tokio::test]
async fn quota_failures_roll_back_and_concurrent_commits_cannot_oversubscribe() {
    let app = TestApp::spawn().await;
    let service = app.state.artifact_service.clone();
    let scope = service
        .resolve_scope(&owner("limited"))
        .await
        .expect("limited scope");
    let scope_id = scope.id;
    service
        .set_limits(
            scope_id,
            StorageLimits {
                artifact_limit: 1,
                byte_limit: 3,
                warning_ratio: 0.8,
            },
        )
        .await
        .expect("set limits");

    let a = service
        .store()
        .stage(Cursor::new(b"aa"), StageLimits::default())
        .expect("stage a");
    let b = service
        .store()
        .stage(Cursor::new(b"bb"), StageLimits::default())
        .expect("stage b");
    let left = {
        let service = service.clone();
        tokio::spawn(async move { service.commit(scope_id, a, metadata("a.csv")).await })
    };
    let right = {
        let service = service.clone();
        tokio::spawn(async move { service.commit(scope_id, b, metadata("b.csv")).await })
    };
    let results = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ArtifactError::QuotaExceeded)))
            .count(),
        1
    );
    let usage = service.get_usage(scope_id).await.expect("usage");
    assert_eq!(usage.used_artifacts, 1);
    assert_eq!(usage.used_bytes, 2);

    let too_large = service
        .store()
        .stage(Cursor::new(b"four"), StageLimits::default())
        .expect("stage too large");
    assert!(matches!(
        service
            .commit(scope_id, too_large, metadata("four.csv"))
            .await,
        Err(ArtifactError::QuotaExceeded)
    ));
}

#[tokio::test]
async fn object_paths_stay_inside_data_dir_and_gc_removes_unreferenced_blobs() {
    let app = TestApp::spawn().await;
    let service = &app.state.artifact_service;
    let scope = service.resolve_scope(&owner("gc")).await.expect("gc scope");
    let artifact = service
        .commit(
            scope.id,
            service
                .store()
                .stage(Cursor::new(b"collect me"), StageLimits::default())
                .expect("stage"),
            metadata("../../escape.csv"),
        )
        .await
        .expect("commit");
    let path = service
        .store()
        .resolve(&artifact.relative_path)
        .expect("safe relative path");
    assert!(path.starts_with(service.store().data_dir()));
    assert!(path.is_file());

    service
        .release(scope.id, artifact.id)
        .await
        .expect("release reference");
    let report = service.collect_unreferenced().await.expect("collect");
    assert_eq!(report.removed_artifacts, 1);
    assert!(!path.exists());
    assert_eq!(service.get_usage(scope.id).await.unwrap().used_artifacts, 0);
}

#[test]
fn staging_enforces_stream_limit_and_cleans_partial_file() {
    let root = tempfile::tempdir().expect("temp root");
    let store =
        backend::storage::ArtifactStore::new(root.path().to_path_buf()).expect("artifact store");
    let error = store
        .stage(Cursor::new(vec![7_u8; 9]), StageLimits { max_bytes: 8 })
        .expect_err("stream must stop at limit");
    assert_eq!(error.code(), "artifact_too_large");
    assert_eq!(
        std::fs::read_dir(root.path().join("staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn storage_usage_is_resolved_from_the_authenticated_principal() {
    let app = TestApp::spawn().await;
    let response = app
        .router
        .oneshot(
            Request::get("/api/storage-usage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("storage usage response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["scope"], "instance");
    assert_eq!(body["artifactCount"], 0);
    assert_eq!(body["artifactLimit"], 20_000);
    assert_eq!(body["logicalBytes"], 0);
    assert_eq!(body["byteLimit"], 20_i64 * 1024 * 1024 * 1024);
    assert_eq!(body["warningRatio"], 0.8);
    assert_eq!(body["isWarning"], false);
    assert_eq!(body["isExceeded"], false);
}
