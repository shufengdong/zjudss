mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::auth::AuthConfig;
use diesel::RunQueryDsl;
use diesel::sql_types::{Integer, Text};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::TestApp;

const TOKEN: &str = "test-proxy-token";

#[derive(diesel::QueryableByName)]
struct ColumnRow {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(diesel::QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Integer)]
    id: i32,
}

fn request(
    method: &str,
    uri: &str,
    subject: &str,
    body: Body,
    content_type: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-zjudss-subject", subject)
        .header("x-zjudss-proxy-token", TOKEN);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    builder.body(body).expect("build request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("parse response json")
}

async fn create_session(app: &TestApp, subject: &str, title: &str) -> i32 {
    let response = app
        .router
        .clone()
        .oneshot(request(
            "POST",
            "/api/sessions",
            subject,
            Body::from(json!({"title": title}).to_string()),
            Some("application/json"),
        ))
        .await
        .expect("create session");
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("numeric session id")
}

#[tokio::test]
async fn migration_requires_session_scope() {
    let app = TestApp::spawn().await;
    let mut conn = app.pool.get().expect("checkout database");

    let storage_table: Vec<IdRow> = diesel::sql_query(
        "SELECT 1 AS id FROM sqlite_master WHERE type='table' AND name='storage_scopes'",
    )
    .load(&mut conn)
    .expect("query storage_scopes table");
    assert_eq!(storage_table.len(), 1);

    let columns: Vec<ColumnRow> =
        diesel::sql_query("SELECT name FROM pragma_table_info('sessions')")
            .load(&mut conn)
            .expect("query session columns");
    let names: Vec<&str> = columns.iter().map(|row| row.name.as_str()).collect();
    assert!(names.contains(&"access_scope_kind"));
    assert!(names.contains(&"access_scope_key"));
    assert!(names.contains(&"storage_scope_id"));

    let insert = diesel::sql_query(
        "INSERT INTO sessions(title, created_at_ms, updated_at_ms) VALUES ('unsafe', 1, 1)",
    )
    .execute(&mut conn);
    assert!(insert.is_err(), "database must reject an unscoped session");
}

#[tokio::test]
async fn trusted_proxy_scopes_all_session_resources() {
    let app = TestApp::spawn_with_auth(AuthConfig::trusted_proxy(TOKEN)).await;
    let alice_session = create_session(&app, "alice@example.com", "Alice").await;
    let bob_session = create_session(&app, "bob@example.com", "Bob").await;

    let alice_list = app
        .router
        .clone()
        .oneshot(request(
            "GET",
            "/api/sessions",
            "alice@example.com",
            Body::empty(),
            None,
        ))
        .await
        .expect("list alice sessions");
    let alice_json = json_body(alice_list).await;
    assert_eq!(
        alice_json["sessions"].as_array().expect("sessions").len(),
        1
    );
    assert_eq!(alice_json["sessions"][0]["id"], alice_session.to_string());

    let bob_list = app
        .router
        .clone()
        .oneshot(request(
            "GET",
            "/api/sessions",
            "bob@example.com",
            Body::empty(),
            None,
        ))
        .await
        .expect("list bob sessions");
    let bob_json = json_body(bob_list).await;
    assert_eq!(bob_json["sessions"].as_array().expect("sessions").len(), 1);
    assert_eq!(bob_json["sessions"][0]["id"], bob_session.to_string());

    let mut conn = app.pool.get().expect("checkout database");
    diesel::sql_query("INSERT INTO artifacts(sha256,size_bytes,storage_path,format,created_at_ms) VALUES('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,'objects/bb/auth-script','rss',1)")
    .execute(&mut conn).expect("insert script artifact");
    let artifact: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .unwrap();
    diesel::sql_query(format!(
        "INSERT INTO workspaces(session_id,created_at_ms,updated_at_ms) VALUES({alice_session},1,1)"
    ))
    .execute(&mut conn)
    .expect("insert workspace");
    let workspace: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .unwrap();
    diesel::sql_query(format!("INSERT INTO model_versions(workspace_id,status,source_kind,origin_kind,manifest_json,content_hash,created_at_ms) VALUES({},'ready','rss','generated','{{}}','auth-version',1)",workspace.id))
    .execute(&mut conn).expect("insert model version");
    let version: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .unwrap();
    diesel::sql_query(format!(
        "INSERT INTO simulation_tasks(session_id,status,stage,study_type,model_version_id,study_spec_json,script_artifact_id,agent_bundle_version,agent_skill_refs_json,engine_contract_version,created_at_ms) \
         VALUES ({alice_session},'succeeded',NULL,'power_flow',{},'{{}}',{},'v2.0.0','[]','test-contract',1)",version.id,artifact.id
    ))
    .execute(&mut conn)
    .expect("insert alice task");
    let task: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .expect("read task id");
    drop(conn);

    let protected = [
        (
            "GET",
            format!("/api/sessions/{alice_session}/messages"),
            None,
        ),
        ("GET", format!("/api/sessions/{alice_session}/events"), None),
        ("GET", format!("/api/tasks/{}", task.id), None),
        (
            "POST",
            format!("/api/sessions/{alice_session}/attachments"),
            Some("multipart/form-data; boundary=x"),
        ),
    ];
    for (method, uri, content_type) in protected {
        let response = app
            .router
            .clone()
            .oneshot(request(
                method,
                &uri,
                "bob@example.com",
                Body::from("--x--\r\n"),
                content_type,
            ))
            .await
            .expect("request protected resource");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
    }

    let hidden = app
        .router
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/sessions/{alice_session}/messages"),
            "bob@example.com",
            Body::empty(),
            None,
        ))
        .await
        .expect("read hidden resource");
    let missing = app
        .router
        .clone()
        .oneshot(request(
            "GET",
            "/api/sessions/2147483647/messages",
            "bob@example.com",
            Body::empty(),
            None,
        ))
        .await
        .expect("read missing resource");
    assert_eq!(hidden.status(), missing.status());
    assert_eq!(json_body(hidden).await, json_body(missing).await);
}

#[tokio::test]
async fn trusted_proxy_rejects_missing_or_invalid_identity() {
    let app = TestApp::spawn_with_auth(AuthConfig::trusted_proxy(TOKEN)).await;

    for request in [
        Request::builder()
            .uri("/api/me")
            .body(Body::empty())
            .expect("missing headers request"),
        Request::builder()
            .uri("/api/me")
            .header("x-zjudss-subject", "alice@example.com")
            .header("x-zjudss-proxy-token", "wrong")
            .body(Body::empty())
            .expect("invalid token request"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("auth response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn me_returns_stable_scope_identity() {
    let app = TestApp::spawn_with_auth(AuthConfig::trusted_proxy(TOKEN)).await;
    let response = app
        .router
        .clone()
        .oneshot(request(
            "GET",
            "/api/me",
            "alice@example.com",
            Body::empty(),
            None,
        ))
        .await
        .expect("me response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        json!({
            "subject": "alice@example.com",
            "scopeKind": "owner",
            "scopeKey": "owner:alice@example.com"
        })
    );
}
