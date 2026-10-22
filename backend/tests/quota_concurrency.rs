mod common;

use std::io::Cursor;

use backend::application::{ArtifactError, ArtifactMetadata, StorageLimits};
use backend::auth::StorageScopeRef;
use backend::storage::StageLimits;
use diesel::prelude::*;

use common::TestApp;

fn metadata() -> ArtifactMetadata {
    ArtifactMetadata {
        format: "bin".into(),
        media_type: Some("application/octet-stream".into()),
    }
}

#[tokio::test]
async fn production_limits_include_four_engine_slots_and_twenty_gib_storage() {
    let app = TestApp::spawn().await;
    assert_eq!(app.state.semaphore.available_permits(), 4);
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&StorageScopeRef {
            kind: "owner".into(),
            key: "defaults".into(),
        })
        .await
        .unwrap();
    let usage = app
        .state
        .artifact_service
        .get_usage(scope.id)
        .await
        .unwrap();
    assert_eq!(usage.artifact_limit, 20_000);
    assert_eq!(usage.byte_limit, 20 * 1024 * 1024 * 1024);
    assert_eq!(usage.warning_ratio, 0.8);
}

#[tokio::test]
async fn concurrent_same_hash_commits_charge_once_and_preserve_refs_at_quota() {
    let app = TestApp::spawn().await;
    let service = app.state.artifact_service.clone();
    let reference = StorageScopeRef {
        kind: "owner".into(),
        key: "concurrent-quota".into(),
    };
    let scope = service.resolve_scope(&reference).await.unwrap();
    service
        .set_limits(
            scope.id,
            StorageLimits {
                artifact_limit: 2,
                byte_limit: 64,
                warning_ratio: 0.5,
            },
        )
        .await
        .unwrap();

    let left = service
        .store()
        .stage(Cursor::new(b"same-content"), StageLimits::default())
        .unwrap();
    let right = service
        .store()
        .stage(Cursor::new(b"same-content"), StageLimits::default())
        .unwrap();
    let first = {
        let service = service.clone();
        tokio::spawn(async move { service.commit(scope.id, left, metadata()).await })
    };
    let second = {
        let service = service.clone();
        tokio::spawn(async move { service.commit(scope.id, right, metadata()).await })
    };
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first.id, second.id);

    let summary = service.usage_for(&reference).await.unwrap();
    assert_eq!(summary.artifact_count, 1);
    assert_eq!(summary.logical_bytes, 12);
    assert!(summary.is_warning);
    let mut conn = app.pool.get().unwrap();
    let refs: i64 = backend::db::schema::storage_scope_artifacts::table
        .find((scope.id, first.id))
        .select(backend::db::schema::storage_scope_artifacts::ref_count)
        .first(&mut conn)
        .unwrap();
    assert_eq!(refs, 2);
    drop(conn);

    let distinct = service
        .store()
        .stage(Cursor::new(b"distinct"), StageLimits::default())
        .unwrap();
    service
        .commit(scope.id, distinct, metadata())
        .await
        .unwrap();
    let rejected = service
        .store()
        .stage(Cursor::new(b"third"), StageLimits::default())
        .unwrap();
    assert!(matches!(
        service.commit(scope.id, rejected, metadata()).await,
        Err(ArtifactError::QuotaExceeded)
    ));
    let usage = service.get_usage(scope.id).await.unwrap();
    assert_eq!(usage.used_artifacts, 2);
    assert_eq!(usage.used_bytes, 20);
}
