mod common;

use std::io::Cursor;

use diesel::prelude::*;
use zjudss::application::{ArtifactError, ArtifactMetadata, StorageLimits};
use zjudss::db::repositories::{RepoError, UploadRepo};
use zjudss::storage::StageLimits;

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
    let scope = app.state.artifact_service.resolve_scope(2).await.unwrap();
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
    let scope = service.resolve_scope(2).await.unwrap();
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

    let summary = service.usage_for(scope.id).await.unwrap();
    assert_eq!(summary.artifact_count, 1);
    assert_eq!(summary.logical_bytes, 12);
    assert!(summary.is_warning);
    let mut conn = app.pool.get().unwrap();
    let refs: i64 = zjudss::db::schema::storage_scope_artifacts::table
        .find((scope.id, first.id))
        .select(zjudss::db::schema::storage_scope_artifacts::ref_count)
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

#[tokio::test]
async fn concurrent_upload_reservations_never_overbook_the_scope() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let left_session = app
        .state
        .session_repo
        .create(&user.id, "left", 1)
        .await
        .unwrap();
    let right_session = app
        .state
        .session_repo
        .create(&user.id, "right", 1)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            user.storage_scope_id,
            StorageLimits {
                artifact_limit: 100,
                byte_limit: 10,
                warning_ratio: 0.8,
            },
        )
        .await
        .unwrap();
    let repo = UploadRepo::new(app.pool.clone());
    let reserve = |repo: UploadRepo, session_id, client: &str| {
        let user_id = user.id.clone();
        let client = client.to_string();
        tokio::spawn(async move {
            repo.create_reserved(
                &user_id,
                user.storage_scope_id,
                session_id,
                client,
                "upload".into(),
                None,
                8,
                i64::MAX,
                1,
            )
            .await
        })
    };
    let left = reserve(repo.clone(), left_session.id, "left");
    let right = reserve(repo, right_session.id, "right");
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(outcomes.iter().filter(|value| value.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| matches!(value, Err(RepoError::QuotaExceeded)))
            .count(),
        1
    );
    let mut conn = app.pool.get().unwrap();
    let reserved: i64 = zjudss::db::schema::storage_scopes::table
        .find(user.storage_scope_id)
        .select(zjudss::db::schema::storage_scopes::reserved_bytes)
        .first(&mut conn)
        .unwrap();
    assert_eq!(reserved, 8);
}
