mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::DraftReadExpectation;
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;

async fn import_table(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    expected: Option<DraftRef>,
    csv: &[u8],
) -> DraftRef {
    common::publish_model_bytes(
        app,
        user,
        session_id,
        client_id,
        "bus.csv",
        csv,
        expected,
        Some("bus"),
    )
    .await
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn drafts_publish_idempotently_deduplicate_and_activate_explicitly() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "versions", 1)
        .await
        .unwrap();
    let draft = import_table(
        &app,
        &user,
        session.id,
        "first",
        None,
        b"id,name\n1,source\n",
    )
    .await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/sessions/{}/workspace", session.id))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let workspace = json_body(response).await;
    assert!(workspace["active"].is_null());
    assert_eq!(workspace["draft"]["id"], draft.id.to_string());
    assert_eq!(workspace["draft"]["revision"], 1);
    assert_eq!(workspace["draft"]["tables"][0]["tableName"], "bus");

    let publish_body = json!({"expectedDraftId":draft.id,"expectedDraftRevision":draft.revision,"clientPublishId":"publish-first"});
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/versions", session.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(publish_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let published = json_body(response).await;
    let first_id = published["version"]["id"].as_str().unwrap().to_string();
    assert_eq!(published["deduplicated"], false);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/versions", session.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(publish_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["version"]["id"], first_id);

    let second_draft = import_table(
        &app,
        &user,
        session.id,
        "same-content",
        None,
        b"id,name\n1,source\n",
    )
    .await;
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/versions", session.id)).header("cookie", &cookie).header("content-type", "application/json").body(Body::from(json!({"expectedDraftId":second_draft.id,"expectedDraftRevision":second_draft.revision,"clientPublishId":"publish-duplicate"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let duplicate = json_body(response).await;
    assert_eq!(duplicate["version"]["id"], first_id);
    assert_eq!(duplicate["deduplicated"], true);

    let changed = import_table(
        &app,
        &user,
        session.id,
        "changed",
        None,
        b"id,name\n2,replacement\n",
    )
    .await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/versions/{first_id}/activate"))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"discardDraft":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = app.router.clone().oneshot(Request::post(format!("/api/versions/{first_id}/activate")).header("cookie", &cookie).header("content-type", "application/json").body(Body::from(json!({"discardDraft":true,"expectedDraft":{"id":changed.id,"revision":changed.revision}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let activated = json_body(response).await;
    assert_eq!(activated["active"]["id"], first_id);
    assert!(activated["draft"].is_null());
}

#[tokio::test]
async fn workspace_and_versions_are_user_scoped_and_publish_checks_revision() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let foreign_cookie = app.login_cookie("bob@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "scope", 1)
        .await
        .unwrap();
    let draft = import_table(
        &app,
        &user,
        session.id,
        "scope",
        None,
        b"id,name\n1,scope\n",
    )
    .await;
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/versions", session.id)).header("cookie", &cookie).header("content-type", "application/json").body(Body::from(json!({"expectedDraftId":draft.id,"expectedDraftRevision":draft.revision+1,"clientPublishId":"stale"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    for path in [
        format!("/api/sessions/{}/workspace", session.id),
        format!("/api/sessions/{}/versions", session.id),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(path)
                    .header("cookie", &foreign_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn session_deletion_preview_tracks_draft_revision_and_deleting_hides_draft_reads() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "draft deletion", 1)
        .await
        .unwrap();
    let first = import_table(
        &app,
        &user,
        session.id,
        "draft-first",
        None,
        b"id,name\n1,source\n",
    )
    .await;
    let stale_token = "a".repeat(64);
    app.state
        .session_repo
        .preview_deletion(&user.id, session.id, stale_token.clone(), i64::MAX, 2)
        .await
        .unwrap();

    let second = import_table(
        &app,
        &user,
        session.id,
        "draft-second",
        Some(first),
        b"id,name\n1,source\n",
    )
    .await;
    let stale = app
        .state
        .session_repo
        .begin_delete(&user.id, session.id, stale_token, 3)
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("deletion_impact_changed"));

    let current_token = "b".repeat(64);
    app.state
        .session_repo
        .preview_deletion(&user.id, session.id, current_token.clone(), i64::MAX, 4)
        .await
        .unwrap();
    let snapshot = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    let dataframe_id = snapshot.draft.as_ref().unwrap().dataframes[0].id;
    assert!(
        app.state
            .session_repo
            .begin_delete(&user.id, session.id, current_token, 5)
            .await
            .unwrap()
    );
    assert!(
        app.state
            .version_service
            .workspace(&user, session.id)
            .await
            .is_err()
    );
    let error = app
        .state
        .model_dataframe_read_service
        .draft_metadata(
            &user,
            dataframe_id,
            DraftReadExpectation {
                draft_id: second.id,
                expected_revision: second.revision,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "not_found");
}
