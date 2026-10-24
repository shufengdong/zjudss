mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::RunQueryDsl;
use diesel::connection::SimpleConnection;
use std::io::Cursor;
use tower::ServiceExt;
use zjudss::application::{UploadBatch, UploadFile};
use zjudss::storage::StageLimits;

use common::TestApp;

fn multipart(client_id: &str, bytes: &str) -> (String, String) {
    let boundary = format!("v3-model-import-{client_id}");
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"clientImportId\"\r\n\r\n{client_id}\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file.bus\"; filename=\"bus.csv\"\r\nContent-Type: text/csv\r\n\r\n{bytes}\r\n\
         --{boundary}--\r\n"
    );
    (boundary, body)
}

#[derive(diesel::QueryableByName)]
struct RefCountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    ref_count: i64,
}

#[tokio::test]
async fn repeated_discard_releases_a_shared_source_artifact_only_once() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "discard-idempotency", 1)
        .await
        .unwrap();
    for client_id in ["first", "second"] {
        let staged = app
            .state
            .artifact_service
            .store()
            .stage(
                Cursor::new(b"id,name\n1,bus\n".to_vec()),
                StageLimits::default(),
            )
            .unwrap();
        app.state
            .model_import_service
            .accept(
                &user,
                session.id,
                client_id.into(),
                None,
                UploadBatch {
                    client_batch_id: "unused".into(),
                    files: vec![UploadFile {
                        client_file_id: "bus".into(),
                        file_name: "bus.csv".into(),
                        relative_path: Some("bus.csv".into()),
                        media_type: Some("text/csv".into()),
                        staged,
                    }],
                },
            )
            .await
            .unwrap();
    }
    app.state
        .model_import_service
        .discard(&user, 1)
        .await
        .unwrap();
    app.state
        .model_import_service
        .discard(&user, 1)
        .await
        .unwrap();

    let mut conn = app.pool.get().unwrap();
    let row = diesel::sql_query(
        "SELECT ref_count FROM storage_scope_artifacts WHERE storage_scope_id=?1",
    )
    .bind::<diesel::sql_types::Integer, _>(user.storage_scope_id)
    .get_result::<RefCountRow>(&mut conn)
    .unwrap();
    assert_eq!(row.ref_count, 1);
}

#[tokio::test]
async fn dss_import_uses_adapter_tables_and_multiple_roots_wait_for_entry_choice() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "dss", 1)
        .await
        .unwrap();
    let file = |id: &str, name: &str, bytes: &[u8]| UploadFile {
        client_file_id: id.into(),
        file_name: name.into(),
        relative_path: Some(name.into()),
        media_type: Some("text/plain".into()),
        staged: app
            .state
            .artifact_service
            .store()
            .stage(Cursor::new(bytes.to_vec()), StageLimits::default())
            .unwrap(),
    };
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session.id,
            "single-dss".into(),
            None,
            UploadBatch {
                client_batch_id: "unused".into(),
                files: vec![file("master", "master.dss", b"New Circuit.Demo\n")],
            },
        )
        .await
        .unwrap();
    let ready = app
        .state
        .model_import_service
        .process(&user, accepted.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status, "ready_to_apply");
    assert_eq!(ready.candidates.len(), 21);
    assert!(
        ready
            .candidates
            .iter()
            .all(|candidate| candidate.kind == "table")
    );

    let multiple = app
        .state
        .model_import_service
        .accept(
            &user,
            session.id,
            "multiple-dss".into(),
            None,
            UploadBatch {
                client_batch_id: "unused".into(),
                files: vec![
                    file("a", "a.dss", b"New Circuit.A\n"),
                    file("b", "b.dss", b"New Circuit.B\n"),
                ],
            },
        )
        .await
        .unwrap();
    let awaiting = app
        .state
        .model_import_service
        .process(&user, multiple.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(awaiting.status, "awaiting_resolution");
    assert_eq!(
        awaiting.diagnostics["entryCandidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        app.state
            .model_import_service
            .get(&user, multiple.id.parse().unwrap())
            .await
            .unwrap()
            .base_draft
            .is_none()
    );
}

async fn create_session(app: &TestApp, cookie: &str) -> String {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"imports"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn upload(
    app: &TestApp,
    cookie: &str,
    session_id: &str,
    client_id: &str,
    bytes: &str,
) -> axum::response::Response {
    let (boundary, body) = multipart(client_id, bytes);
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session_id}/model-imports"))
                .header("cookie", cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn upload_is_accepted_idempotent_and_persists_resolution_state() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice@example.com").await;
    let session_id = create_session(&app, &cookie).await;
    let first = upload(&app, &cookie, &session_id, "client-one", "id,name\n1,bus\n").await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: serde_json::Value =
        serde_json::from_slice(&to_bytes(first.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let import_id = first["modelImport"]["id"].as_str().unwrap();
    assert_eq!(first["modelImport"]["status"], "accepted");
    assert_eq!(
        first["modelImport"]["candidates"].as_array().unwrap().len(),
        1
    );
    assert!(
        !first["modelImport"]["candidates"][0]["id"]
            .as_str()
            .unwrap()
            .contains('/')
    );

    let retry = upload(&app, &cookie, &session_id, "client-one", "id,name\n1,bus\n").await;
    assert_eq!(retry.status(), StatusCode::ACCEPTED);
    let retry: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(retry["modelImport"]["id"], import_id);

    let conflict = upload(
        &app,
        &cookie,
        &session_id,
        "client-one",
        "id,name\n2,line\n",
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(
                    Request::get(format!("/api/model-imports/{import_id}"))
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            if matches!(
                value["modelImport"]["status"].as_str(),
                Some("ready_to_apply" | "failed")
            ) {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background import persists a recoverable state");
    assert_eq!(
        terminal["modelImport"]["status"], "ready_to_apply",
        "{terminal}"
    );
    assert_eq!(
        terminal["modelImport"]["diagnostics"]["resolution"],
        "confirm_apply"
    );
    let candidate_id = terminal["modelImport"]["candidates"][0]["id"]
        .as_str()
        .unwrap();
    let applied = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/model-imports/{import_id}/resolve"))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"candidateIds":[candidate_id]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(applied.status(), StatusCode::OK);
    let applied: serde_json::Value =
        serde_json::from_slice(&to_bytes(applied.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(applied["modelImport"]["status"], "applied");
    assert_eq!(applied["draft"]["revision"], 1);
}

#[tokio::test]
async fn imports_are_user_scoped_discardable_and_compare_empty_draft_identity() {
    let app = TestApp::spawn().await;
    let alice = app.login_cookie("alice@example.com").await;
    let bob = app.login_cookie("bob@example.com").await;
    let session_id = create_session(&app, &alice).await;
    let first = upload(&app, &alice, &session_id, "first", "id\n1\n").await;
    let first: serde_json::Value =
        serde_json::from_slice(&to_bytes(first.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let import_id = first["modelImport"]["id"].as_str().unwrap();

    let hidden = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/model-imports/{import_id}"))
                .header("cookie", &bob)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let discarded = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/model-imports/{import_id}"))
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(discarded.status(), StatusCode::NO_CONTENT);

    let mut conn = app.pool.get().unwrap();
    conn.batch_execute(
        "INSERT INTO workspace_drafts(workspace_id,revision,diagnostics_json,created_at_ms,updated_at_ms)
         SELECT id,1,'{}',1,1 FROM workspaces WHERE session_id=1",
    )
    .unwrap();
    let stale = upload(&app, &alice, &session_id, "stale", "id\n2\n").await;
    let stale_status = stale.status();
    let stale: serde_json::Value =
        serde_json::from_slice(&to_bytes(stale.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(stale_status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "draft_revision_conflict");

    let discarded = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/model-imports/{import_id}"))
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let discarded: serde_json::Value =
        serde_json::from_slice(&to_bytes(discarded.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(discarded["modelImport"]["status"], "discarded");
}

#[tokio::test]
async fn service_restart_fails_processing_import_but_keeps_persisted_candidates() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "restart", 1)
        .await
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session.id,
            "restart-import".into(),
            None,
            UploadBatch {
                client_batch_id: "unused".into(),
                files: vec![UploadFile {
                    client_file_id: "bus".into(),
                    file_name: "bus.csv".into(),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged: app
                        .state
                        .artifact_service
                        .store()
                        .stage(Cursor::new(b"id\n1\n".to_vec()), StageLimits::default())
                        .unwrap(),
                }],
            },
        )
        .await
        .unwrap();
    let import_id = accepted.id.parse::<i32>().unwrap();
    let mut conn = app.pool.get().unwrap();
    conn.batch_execute(&format!(
        "UPDATE model_imports SET status='processing' WHERE id={import_id}"
    ))
    .unwrap();
    zjudss::db::recover_interrupted_runs(&mut conn, 99).unwrap();
    let recovered = app
        .state
        .model_import_service
        .get(&user, import_id)
        .await
        .unwrap();
    assert_eq!(recovered.status, "failed");
    assert_eq!(recovered.error_code.as_deref(), Some("service_restarted"));
    assert_eq!(recovered.candidates.len(), 1);
}
