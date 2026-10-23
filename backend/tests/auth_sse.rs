mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use backend::application::now_ms;
use backend::auth::UserId;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use common::TestApp;

#[derive(QueryableByName)]
struct SessionTimes {
    #[diesel(sql_type = BigInt)]
    last_seen_at_ms: i64,
    #[diesel(sql_type = BigInt)]
    expires_at_ms: i64,
}

fn token_hash(cookie: &str) -> String {
    let token = cookie
        .strip_prefix("zjudss_session=")
        .expect("session cookie");
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn session_times(app: &TestApp, hash: &str) -> SessionTimes {
    let mut conn = app.pool.get().expect("database");
    diesel::sql_query("SELECT last_seen_at_ms,expires_at_ms FROM auth_sessions WHERE token_hash=?")
        .bind::<Text, _>(hash)
        .get_result(&mut conn)
        .expect("auth session times")
}

fn set_expiry(app: &TestApp, hash: &str, expires_at_ms: i64) {
    let mut conn = app.pool.get().expect("database");
    diesel::sql_query("UPDATE auth_sessions SET expires_at_ms=? WHERE token_hash=?")
        .bind::<BigInt, _>(expires_at_ms)
        .bind::<Text, _>(hash)
        .execute(&mut conn)
        .expect("set auth session expiry");
}

async fn open_stream(app: &TestApp, cookie: &str, session_id: i32) -> axum::response::Response {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/sessions/{session_id}/events"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("open SSE");
    assert_eq!(response.status(), StatusCode::OK);
    response
}

async fn logout(app: &TestApp, cookie: &str) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/auth/logout")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("logout");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

async fn stream_ends(response: axum::response::Response) -> bool {
    let mut stream = response.into_body().into_data_stream();
    tokio::time::timeout(Duration::from_secs(2), async move {
        while stream.next().await.is_some() {}
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn heartbeat_is_read_only_after_connection_renewal() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice").await;
    let hash = token_hash(&cookie);
    let session = app
        .state
        .session_repo
        .create(&UserId("alice".into()), "SSE auth", now_ms())
        .await
        .unwrap();
    set_expiry(&app, &hash, now_ms() + 1_000);

    let response = open_stream(&app, &cookie, session.id).await;
    assert!(response.headers().contains_key(header::SET_COOKIE));
    let renewed = session_times(&app, &hash);
    let mut stream = response.into_body().into_data_stream();
    tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("heartbeat arrives")
        .expect("stream remains open")
        .expect("heartbeat chunk");
    let after_heartbeat = session_times(&app, &hash);
    assert_eq!(after_heartbeat.last_seen_at_ms, renewed.last_seen_at_ms);
    assert_eq!(after_heartbeat.expires_at_ms, renewed.expires_at_ms);
}

#[tokio::test]
async fn expiry_ends_an_open_stream() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice").await;
    let hash = token_hash(&cookie);
    let session = app
        .state
        .session_repo
        .create(&UserId("alice".into()), "SSE expiry", now_ms())
        .await
        .unwrap();
    let response = open_stream(&app, &cookie, session.id).await;

    set_expiry(&app, &hash, 0);
    assert!(stream_ends(response).await, "expired SSE must terminate");
}

#[tokio::test]
async fn logout_ends_only_the_matching_browser_stream() {
    let app = TestApp::spawn().await;
    let first = app.login_cookie("alice").await;
    let second = app.login_cookie("alice").await;
    let session = app
        .state
        .session_repo
        .create(&UserId("alice".into()), "SSE logout", now_ms())
        .await
        .unwrap();

    let first_response = open_stream(&app, &first, session.id).await;
    logout(&app, &second).await;
    let mut first_stream = first_response.into_body().into_data_stream();
    tokio::time::timeout(Duration::from_secs(1), first_stream.next())
        .await
        .expect("unrelated logout leaves stream live")
        .expect("first stream remains open")
        .expect("heartbeat chunk");

    let matching_response = open_stream(&app, &first, session.id).await;
    logout(&app, &first).await;
    assert!(
        stream_ends(matching_response).await,
        "matching logout must terminate SSE"
    );
}
