mod common;

use std::io::Cursor;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{
    ArtifactMetadata, DataframeBindingDraft, ModelVersionStatus, SourceKind, VersionDraft,
    VersionInputDraft, VersionOrigin, WorkspaceError, canonical_json,
};
use backend::auth::{AccessScope, StorageScopeRef};
use backend::storage::StageLimits;
use diesel::RunQueryDsl;
use serde_json::json;
use tower::ServiceExt;

use common::TestApp;

const TOKEN: &str = "workspace-test-token";

fn scope(subject: &str) -> AccessScope {
    AccessScope {
        kind: "owner".to_string(),
        key: format!("owner:{subject}"),
    }
}

fn draft(tag: &str, origin: VersionOrigin) -> VersionDraft {
    VersionDraft {
        status: ModelVersionStatus::Ready,
        source_kind: SourceKind::Dataframes,
        origin,
        manifest: json!({
            "diagnostics": [],
            "tag": tag,
            "missing": [],
            "inputs": []
        }),
        patch: None,
        inputs: vec![],
    }
}

#[test]
fn canonical_json_sorts_object_keys_recursively() {
    assert_eq!(
        canonical_json(&json!({"z": 1, "a": {"y": 2, "b": 3}})),
        r#"{"a":{"b":3,"y":2},"z":1}"#
    );
}

#[tokio::test]
async fn ready_versions_follow_active_draft_activation_and_rollback_rules() {
    let app = TestApp::spawn().await;
    let access = AccessScope {
        kind: "instance".into(),
        key: "instance".into(),
    };
    let session = app
        .state
        .session_repo
        .create(&access, "workspace", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&access, session.id)
        .await
        .unwrap();

    let first = app
        .state
        .workspace_service
        .publish_version(
            &access,
            workspace.workspace.id,
            None,
            draft("first", VersionOrigin::Upload),
        )
        .await
        .unwrap();
    let after_first = app
        .state
        .workspace_service
        .get(&access, session.id)
        .await
        .unwrap();
    assert_eq!(after_first.workspace.active_version_id, Some(first.id));
    assert_eq!(after_first.workspace.draft_version_id, None);

    let example_origin = VersionOrigin::Example {
        example_id: "ex_demo".into(),
        engine_resource_snapshot: "sha256:snapshot".into(),
        source_tree_hash: "sha256:tree".into(),
    };
    let second = app
        .state
        .workspace_service
        .publish_version(
            &access,
            workspace.workspace.id,
            Some(first.id),
            draft("second", example_origin),
        )
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&second.manifest_json).unwrap();
    assert_eq!(manifest["originKind"], "example");
    assert_eq!(manifest["origin"]["exampleId"], "ex_demo");
    let before_activation = app
        .state
        .workspace_service
        .get(&access, session.id)
        .await
        .unwrap();
    assert_eq!(
        before_activation.workspace.active_version_id,
        Some(first.id)
    );
    assert_eq!(
        before_activation.workspace.draft_version_id,
        Some(second.id)
    );

    let conflict = app
        .state
        .workspace_service
        .publish_version(
            &access,
            workspace.workspace.id,
            Some(first.id),
            draft("stale", VersionOrigin::Upload),
        )
        .await
        .expect_err("stale base must fail");
    assert!(matches!(conflict, WorkspaceError::VersionConflict));

    let activated = app
        .state
        .workspace_service
        .activate(&access, workspace.workspace.id, second.id)
        .await
        .unwrap();
    assert_eq!(activated.workspace.active_version_id, Some(second.id));
    assert_eq!(activated.workspace.draft_version_id, None);

    let third = app
        .state
        .workspace_service
        .publish_version(
            &access,
            workspace.workspace.id,
            Some(second.id),
            draft("third", VersionOrigin::Generated),
        )
        .await
        .unwrap();
    let rolled_back = app
        .state
        .workspace_service
        .rollback(&access, workspace.workspace.id, first.id)
        .await
        .unwrap();
    assert_eq!(rolled_back.workspace.active_version_id, Some(first.id));
    assert_eq!(rolled_back.workspace.draft_version_id, Some(third.id));
    assert_eq!(
        app.state
            .workspace_service
            .list_versions(&access, session.id)
            .await
            .unwrap()
            .len(),
        3
    );

    let messages = app
        .state
        .message_repo
        .list_by_session(&access, session.id)
        .await
        .unwrap();
    assert!(messages.iter().any(|message| {
        message.kind == "model-status"
            && message.payload_json.as_deref().is_some_and(|payload| {
                let value: serde_json::Value = serde_json::from_str(payload).unwrap();
                value["scope"] == "draft" && value["snapshot"].is_null()
            })
    }));
    let draft_cleared = messages
        .iter()
        .find(|message| {
            message.kind == "model-status"
                && message.payload_json.as_deref().is_some_and(|payload| {
                    let value: serde_json::Value = serde_json::from_str(payload).unwrap();
                    value["scope"] == "draft" && value["snapshot"].is_null()
                })
        })
        .unwrap();
    let replayed =
        serde_json::to_value(backend::application::AppEvent::from_message(draft_cleared)).unwrap();
    assert_eq!(replayed["type"], "model-status");
    assert!(replayed["snapshot"].is_null());

    let mut conn = app.pool.get().unwrap();
    let mutation = diesel::sql_query(format!(
        "UPDATE model_versions SET status='invalid' WHERE id={}",
        first.id
    ))
    .execute(&mut conn);
    assert!(mutation.is_err(), "published versions must be immutable");
    drop(conn);
    app.state
        .session_repo
        .delete(&access, session.id)
        .await
        .expect("session cleanup must cascade immutable versions");
}

#[tokio::test]
async fn workspace_read_endpoints_hide_foreign_sessions_and_versions() {
    let app = TestApp::spawn().await;
    let alice = scope("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&alice, "alice", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&alice, session.id)
        .await
        .unwrap();
    let version = app
        .state
        .workspace_service
        .publish_version(
            &alice,
            workspace.workspace.id,
            None,
            draft("alice", VersionOrigin::Upload),
        )
        .await
        .unwrap();

    let own_response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/sessions/{}/workspace", session.id))
                .header("x-zjudss-subject", "alice@example.com")
                .header("x-zjudss-proxy-token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(own_response.status(), StatusCode::OK);
    let own_body: serde_json::Value =
        serde_json::from_slice(&to_bytes(own_response.into_body(), 64 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(own_body["active"]["versionId"], version.id.to_string());
    assert_eq!(own_body["active"]["originKind"], "upload");

    for uri in [
        format!("/api/sessions/{}/workspace", session.id),
        format!("/api/sessions/{}/versions", session.id),
        format!("/api/versions/{}", version.id),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(uri)
                    .header("x-zjudss-subject", "bob@example.com")
                    .header("x-zjudss-proxy-token", TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn version_inputs_require_owned_artifacts_and_are_returned_by_the_api() {
    let app = TestApp::spawn().await;
    let alice = scope("alice@example.com");
    let bob_storage = app
        .state
        .artifact_service
        .resolve_scope(&StorageScopeRef {
            kind: "owner".into(),
            key: "owner:bob@example.com".into(),
        })
        .await
        .unwrap();
    let artifact = app
        .state
        .artifact_service
        .commit(
            bob_storage.id,
            app.state
                .artifact_service
                .store()
                .stage(Cursor::new(b"name,p\nload-1,100\n"), StageLimits::default())
                .unwrap(),
            ArtifactMetadata {
                format: "csv".into(),
                media_type: Some("text/csv".into()),
            },
        )
        .await
        .unwrap();
    let session = app
        .state
        .session_repo
        .create(&alice, "alice", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&alice, session.id)
        .await
        .unwrap();
    let with_input = VersionDraft {
        status: ModelVersionStatus::Ready,
        source_kind: SourceKind::Dataframes,
        origin: VersionOrigin::Upload,
        manifest: json!({"missing": [], "diagnostics": []}),
        patch: None,
        inputs: vec![VersionInputDraft {
            artifact_id: artifact.id,
            sha256: artifact.sha256.clone(),
            role: "input".into(),
            relative_path: Some("inputs/load.csv".into()),
            display_name: "load.csv".into(),
            source_artifact_id: None,
            source_artifact_sha256: None,
            sheet_name: None,
            binding: Some(DataframeBindingDraft {
                table_kind: Some("load".into()),
                confidence: Some(0.96),
                resolution: "inferred".into(),
                columns: json!([{"name":"name","dtype":"string"},{"name":"p","dtype":"float64"}]),
                evidence: json!({"candidates":[{"tableKind":"load","confidence":0.96}],"matchedRequiredColumns":["name","p"]}),
            }),
        }],
    };
    let error = app
        .state
        .workspace_service
        .publish_version(&alice, workspace.workspace.id, None, with_input)
        .await
        .expect_err("alice cannot reference bob's artifact");
    assert!(matches!(error, WorkspaceError::NotFound));

    let alice_storage = app
        .state
        .artifact_service
        .resolve_scope(&StorageScopeRef {
            kind: "owner".into(),
            key: "owner:alice@example.com".into(),
        })
        .await
        .unwrap();
    let alice_artifact = app
        .state
        .artifact_service
        .commit(
            alice_storage.id,
            app.state
                .artifact_service
                .store()
                .stage(Cursor::new(b"name,p\nload-1,100\n"), StageLimits::default())
                .unwrap(),
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
            &alice,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({"missing": [], "diagnostics": []}),
                patch: None,
                inputs: vec![VersionInputDraft {
                    artifact_id: alice_artifact.id,
                    sha256: alice_artifact.sha256,
                    role: "input".into(),
                    relative_path: Some("inputs/load.csv".into()),
                    display_name: "load.csv".into(),
                    source_artifact_id: None,
                    source_artifact_sha256: None,
                    sheet_name: None,
                    binding: Some(DataframeBindingDraft {
                        table_kind: Some("load".into()),
                        confidence: Some(0.96),
                        resolution: "inferred".into(),
                        columns: json!([]),
                        evidence: json!({"matchedRequiredColumns":["name","p"]}),
                    }),
                }],
            },
        )
        .await
        .unwrap();
    let response = app
        .router
        .oneshot(
            Request::get(format!("/api/versions/{}", version.id))
                .header("x-zjudss-subject", "alice@example.com")
                .header("x-zjudss-proxy-token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["inputs"][0]["tableKind"], "load");
    assert_eq!(
        body["inputs"][0]["matchedRequiredColumns"],
        json!(["name", "p"])
    );

    let replacement = app
        .state
        .workspace_service
        .publish_version(
            &alice,
            workspace.workspace.id,
            Some(version.id),
            draft("replacement", VersionOrigin::Generated),
        )
        .await
        .unwrap();
    app.state
        .workspace_service
        .activate(&alice, workspace.workspace.id, replacement.id)
        .await
        .unwrap();
    assert!(
        app.state
            .workspace_service
            .references_artifact(&alice, alice_artifact.id)
            .await
            .unwrap(),
        "an artifact referenced only by a historical version must remain owned"
    );
    assert!(
        !app.state
            .workspace_service
            .references_artifact(&scope("bob@example.com"), alice_artifact.id)
            .await
            .unwrap()
    );
}
