mod common;

use backend::application::{
    ExampleImportError, ModelVersionStatus, SourceKind, StorageLimits, VersionDraft, VersionOrigin,
};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use diesel::RunQueryDsl;
use serde_json::json;

use common::TestApp;

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

async fn session(app: &TestApp, title: &str) -> i32 {
    app.state
        .session_repo
        .create(&principal().access, title, 1)
        .await
        .unwrap()
        .id
}

fn example(app: &TestApp) -> (String, String) {
    let snapshot = app.state.example_import_service.snapshot();
    (snapshot.examples[0].id.clone(), snapshot.id.clone())
}

#[tokio::test]
async fn root_rss_import_copies_full_closure_is_idempotent_and_creates_no_task() {
    let app = TestApp::spawn().await;
    let session_id = session(&app, "example").await;
    let (example_id, snapshot_id) = example(&app);
    let version = app
        .state
        .example_import_service
        .import(&principal(), session_id, &example_id, &snapshot_id, None)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get(&principal().access, session_id)
        .await
        .unwrap();
    assert_eq!(workspace.workspace.active_version_id, Some(version.id));
    assert_eq!(version.origin_kind, "example");
    let inputs = app
        .state
        .workspace_service
        .inputs(&principal().access, version.id)
        .await
        .unwrap();
    assert_eq!(
        inputs.len(),
        5,
        "entry plus dss/models, nested DSS, bmopf and rspower dependencies"
    );
    let manifest: serde_json::Value = serde_json::from_str(&version.manifest_json).unwrap();
    assert_eq!(manifest["origin"]["exampleId"], example_id);
    assert!(
        version
            .manifest_json
            .find(
                app.state
                    .example_import_service
                    .snapshot()
                    .engine_root
                    .to_string_lossy()
                    .as_ref()
            )
            .is_none()
    );

    let repeated = app
        .state
        .example_import_service
        .import(&principal(), session_id, &example_id, &snapshot_id, None)
        .await
        .unwrap();
    assert_eq!(repeated.id, version.id);
    let mut conn = app.pool.get().unwrap();
    let task_count: i64 =
        diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT COUNT(*) FROM simulation_tasks")
            .get_result(&mut conn)
            .unwrap();
    assert_eq!(task_count, 0);
}

#[tokio::test]
async fn existing_active_keeps_active_and_import_advances_draft() {
    let app = TestApp::spawn().await;
    let session_id = session(&app, "draft").await;
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&principal().access, session_id)
        .await
        .unwrap();
    let active = app
        .state
        .workspace_service
        .publish_version(
            &principal().access,
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
    let (example_id, snapshot_id) = example(&app);
    let imported = app
        .state
        .example_import_service
        .import(
            &principal(),
            session_id,
            &example_id,
            &snapshot_id,
            Some(active.id),
        )
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get(&principal().access, session_id)
        .await
        .unwrap();
    assert_eq!(workspace.workspace.active_version_id, Some(active.id));
    assert_eq!(workspace.workspace.draft_version_id, Some(imported.id));
}

#[tokio::test]
async fn snapshot_source_scope_and_quota_failures_publish_no_partial_version() {
    let app = TestApp::spawn().await;
    let session_id = session(&app, "failures").await;
    let (example_id, snapshot_id) = example(&app);
    assert!(matches!(
        app.state
            .example_import_service
            .import(&principal(), session_id, &example_id, "wrong", None)
            .await,
        Err(ExampleImportError::SnapshotMismatch)
    ));

    let foreign = Principal {
        access: AccessScope {
            kind: "owner".into(),
            key: "owner:foreign".into(),
        },
        storage: StorageScopeRef {
            kind: "owner".into(),
            key: "owner:foreign".into(),
        },
        subject: "foreign".into(),
    };
    assert!(matches!(
        app.state
            .example_import_service
            .import(&foreign, session_id, &example_id, &snapshot_id, None)
            .await,
        Err(ExampleImportError::Workspace(_))
    ));

    let scope = app
        .state
        .artifact_service
        .resolve_scope(&principal().storage)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            scope.id,
            StorageLimits {
                artifact_limit: 1,
                byte_limit: 1024 * 1024,
                warning_ratio: 0.8,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        app.state
            .example_import_service
            .import(&principal(), session_id, &example_id, &snapshot_id, None)
            .await,
        Err(ExampleImportError::Artifact(
            backend::application::ArtifactError::QuotaExceeded
        ))
    ));
    assert!(
        app.state
            .workspace_service
            .list_versions(&principal().access, session_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        app.state
            .artifact_service
            .get_usage(scope.id)
            .await
            .unwrap()
            .used_artifacts,
        0
    );
}

#[tokio::test]
async fn changed_snapshot_source_is_rejected_before_version_publication() {
    let app = TestApp::spawn().await;
    let session_id = session(&app, "changed").await;
    let (example_id, snapshot_id) = example(&app);
    let source = app
        .state
        .example_import_service
        .snapshot()
        .engine_root
        .join("sparrowzz/dss/demo.rss");
    std::fs::write(source, b"changed").unwrap();
    assert!(matches!(
        app.state
            .example_import_service
            .import(&principal(), session_id, &example_id, &snapshot_id, None)
            .await,
        Err(ExampleImportError::SourceChanged(_))
    ));
    assert!(
        app.state
            .workspace_service
            .list_versions(&principal().access, session_id)
            .await
            .unwrap()
            .is_empty()
    );
}
