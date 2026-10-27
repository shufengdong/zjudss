mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use diesel::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;

fn multipart(client_id: &str, bytes: &str) -> (String, String) {
    let boundary = format!("v4-upload-{client_id}");
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"clientBatchId\"\r\n\r\n{client_id}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file.data\"; filename=\"data.csv\"\r\nContent-Type: text/csv\r\n\r\n{bytes}\r\n--{boundary}--\r\n"
    );
    (boundary, body)
}

async fn create_session(app: &TestApp, cookie: &str) -> String {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"uploads"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn upload(
    app: &TestApp,
    cookie: &str,
    session: &str,
    client: &str,
    bytes: &str,
) -> axum::response::Response {
    let (boundary, body) = multipart(client, bytes);
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session}/upload-batches"))
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
async fn upload_batch_is_idempotent_by_source_and_requires_purpose() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice@example.com").await;
    let session = create_session(&app, &cookie).await;
    let first = upload(&app, &cookie, &session, "client-one", "x,y\n1,2\n").await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: Value =
        serde_json::from_slice(&to_bytes(first.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(first["uploadBatch"]["status"], "awaiting_purpose");
    let id = first["uploadBatch"]["id"].as_str().unwrap();
    let candidate = first["uploadBatch"]["candidates"][0]["id"]
        .as_str()
        .unwrap();
    let reparsed = app
        .router
        .clone()
        .oneshot(
            Request::put(format!("/api/upload-batches/{id}/parse-settings"))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"delimiter":44}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reparsed.status(), StatusCode::OK);
    let retry = upload(&app, &cookie, &session, "client-one", "x,y\n1,2\n").await;
    assert_eq!(retry.status(), StatusCode::ACCEPTED);
    let conflict = upload(&app, &cookie, &session, "client-one", "x,y\n3,4\n").await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let decided=app.router.clone().oneshot(Request::put(format!("/api/upload-batches/{id}/candidates")).header("cookie",&cookie).header("content-type","application/json").body(Body::from(json!({"decisions":[{"candidateId":candidate,"purpose":"data_resource","selected":true,"displayName":"负荷预测","alias":"forecast","tableName":null}]}).to_string())).unwrap()).await.unwrap();
    assert_eq!(decided.status(), StatusCode::OK);
    let decided: Value =
        serde_json::from_slice(&to_bytes(decided.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(decided["uploadBatch"]["status"], "ready_to_confirm");
    assert_eq!(decided["uploadBatch"]["resourcePublishStatus"], "ready");
}

#[tokio::test]
async fn upload_batches_are_user_scoped_and_discard_is_idempotent() {
    let app = TestApp::spawn().await;
    let alice = app.login_cookie("alice@example.com").await;
    let bob = app.login_cookie("bob@example.com").await;
    let session = create_session(&app, &alice).await;
    let response = upload(&app, &alice, &session, "scope-one", "x\n1\n").await;
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let id = body["uploadBatch"]["id"].as_str().unwrap();
    let foreign = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/upload-batches/{id}"))
                .header("cookie", &bob)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    for _ in 0..2 {
        let discarded = app
            .router
            .clone()
            .oneshot(
                Request::delete(format!("/api/upload-batches/{id}"))
                    .header("cookie", &alice)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(discarded.status(), StatusCode::NO_CONTENT);
    }
    let current = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/upload-batches/{id}"))
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(current.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["uploadBatch"]["status"], "discarded");
    assert_eq!(body["uploadBatch"]["reservedBytes"], 0);
}

#[tokio::test]
async fn legacy_model_import_routes_are_removed() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice@example.com").await;
    let session = create_session(&app, &cookie).await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session}/model-imports"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn replacement_batch_uses_the_owned_resource_session() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = create_session(&app, &cookie).await;
    let accepted = upload(&app, &cookie, &session, "replacement-source", "x,y\n1,2\n").await;
    let body: Value =
        serde_json::from_slice(&to_bytes(accepted.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let batch_id = body["uploadBatch"]["id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    let artifact_id = zjudss::db::schema::upload_candidates::table
        .filter(zjudss::db::schema::upload_candidates::batch_id.eq(batch_id))
        .select(zjudss::db::schema::upload_candidates::normalized_artifact_id)
        .first::<Option<i32>>(&mut conn)
        .unwrap()
        .unwrap();
    diesel::insert_into(zjudss::db::schema::data_resources::table)
        .values(zjudss::db::models::DataResourceRecord {
            id: "resource_owned".into(),
            session_id: session.parse().unwrap(),
            display_name: "旧资源".into(),
            alias: "old".into(),
            normalized_alias: "old".into(),
            status: "live".into(),
            declared_schema_ref: None,
            source_kind: "upload".into(),
            normalized_artifact_id: artifact_id,
            source_artifact_id: None,
            schema_json: "[]".into(),
            schema_hash: "0".repeat(64),
            row_count: 1,
            source_batch_id: Some(batch_id),
            source_model_version_id: None,
            replaced_by_resource_id: None,
            lineage_json: "{}".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
            tombstone_json: None,
        })
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let (boundary, multipart) = multipart("replacement-new", "x,y\n3,4\n");
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/data-resources/resource_owned/replacement-batches")
                .header("cookie", cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["uploadBatch"]["sessionId"], session);
    assert_eq!(body["uploadBatch"]["clientBatchId"], "replacement-new");
    assert_eq!(
        body["uploadBatch"]["replacementResourceId"],
        "resource_owned"
    );
    assert_eq!(user.id.0, "alice@example.com");
}
