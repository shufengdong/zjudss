mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{UploadBatch, UploadFile};
use backend::auth::CurrentUser;
use backend::db::models::DraftRef;
use backend::storage::StageLimits;
use common::TestApp;
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;

async fn import_table(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    expected: Option<DraftRef>,
    csv: &[u8],
) -> DraftRef {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(csv), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            user,
            session_id,
            client_id.into(),
            expected,
            UploadBatch {
                client_batch_id: client_id.into(),
                files: vec![UploadFile {
                    client_file_id: "bus".into(),
                    file_name: "bus.csv".into(),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let ready = app
        .state
        .model_import_service
        .process(user, accepted.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status, "ready_to_apply", "{:#?}", ready.diagnostics);
    app.state
        .model_import_service
        .resolve_and_apply(
            user,
            accepted.id.parse().unwrap(),
            expected,
            ready
                .candidates
                .iter()
                .map(|item| item.id.clone())
                .collect(),
        )
        .await
        .unwrap()
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
    let draft = import_table(&app, &user, session.id, "scope", None, b"id\n1\n").await;
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
