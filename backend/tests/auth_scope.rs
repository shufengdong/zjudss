mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use common::TestApp;

#[derive(QueryableByName)]
struct AuthRow {
    #[diesel(sql_type = Text)]
    token_hash: String,
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = BigInt)]
    expires_at_ms: i64,
}

fn request(method: &str, uri: &str, cookie: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .expect("build request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse response json")
}

async fn login(app: &TestApp, id: &str, password: &str) -> (String, Value, String) {
    let response = app
        .router
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/login",
            None,
            json!({"id":id,"password":password}),
        ))
        .await
        .expect("login response");
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie")
        .to_str()
        .expect("cookie text")
        .to_string();
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();
    let body = json_body(response).await;
    (cookie, body, set_cookie)
}

#[tokio::test]
async fn login_sets_persistent_opaque_cookie_and_me_returns_user() {
    let app = TestApp::spawn().await;
    let (cookie, body, set_cookie) = login(&app, "alice", "alice").await;
    assert_eq!(body, json!({"user":{"id":"alice","name":"Alice"}}));
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
        assert!(set_cookie.contains(attribute), "missing {attribute}");
    }
    assert!(!set_cookie.contains("Secure"));
    let raw_token = cookie.strip_prefix("zjudss_session=").expect("cookie name");
    assert!(!body.to_string().contains(raw_token));
    let expected_hash = format!("{:x}", Sha256::digest(raw_token.as_bytes()));
    let mut conn = app.pool.get().expect("database");
    let row = diesel::sql_query(
        "SELECT token_hash,user_id,expires_at_ms FROM auth_sessions WHERE user_id='alice'",
    )
    .get_result::<AuthRow>(&mut conn)
    .expect("stored auth session");
    assert_eq!(row.token_hash, expected_hash);
    assert_ne!(row.token_hash, raw_token);
    assert_eq!(row.user_id, "alice");
    assert!(row.expires_at_ms > backend::application::now_ms());
    drop(conn);
    let response = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&cookie), Value::Null))
        .await
        .expect("me response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(
        json_body(response).await,
        json!({"id":"alice","name":"Alice"})
    );
}

#[tokio::test]
async fn invalid_credentials_and_missing_auth_have_stable_distinct_errors() {
    let app = TestApp::spawn().await;
    let mut failures = Vec::new();
    for (id, password) in [("alice", "wrong"), ("unknown", "unknown")] {
        let response = app
            .router
            .clone()
            .oneshot(request(
                "POST",
                "/api/auth/login",
                None,
                json!({"id":id,"password":password}),
            ))
            .await
            .expect("failed login");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        failures.push(json_body(response).await);
    }
    assert_eq!(failures[0], failures[1]);
    assert_eq!(failures[0]["code"], "invalid_credentials");
    for cookie in [None, Some("zjudss_session=unknown-token")] {
        let response = app
            .router
            .clone()
            .oneshot(request("GET", "/api/sessions", cookie, Value::Null))
            .await
            .expect("protected response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(response).await["code"], "authentication_required");
    }
}

#[tokio::test]
async fn expiring_session_renews_and_expired_session_is_deleted() {
    let app = TestApp::spawn().await;
    let (cookie, _, _) = login(&app, "alice", "alice").await;
    let raw = cookie.strip_prefix("zjudss_session=").unwrap();
    let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let fresh = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&cookie), Value::Null))
        .await
        .expect("fresh response");
    assert!(fresh.headers().get(header::SET_COOKIE).is_none());
    let mut conn = app.pool.get().expect("database");
    let near = backend::application::now_ms() + 86_400_000;
    diesel::sql_query("UPDATE auth_sessions SET expires_at_ms=? WHERE token_hash=?")
        .bind::<BigInt, _>(near)
        .bind::<Text, _>(&hash)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let renewed = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&cookie), Value::Null))
        .await
        .expect("renewal response");
    assert_eq!(renewed.status(), StatusCode::OK);
    assert!(
        renewed.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=2592000")
    );
    let mut conn = app.pool.get().expect("database");
    let row = diesel::sql_query(
        "SELECT token_hash,user_id,expires_at_ms FROM auth_sessions WHERE token_hash=?",
    )
    .bind::<Text, _>(&hash)
    .get_result::<AuthRow>(&mut conn)
    .unwrap();
    assert!(row.expires_at_ms > near);
    diesel::sql_query("UPDATE auth_sessions SET expires_at_ms=0 WHERE token_hash=?")
        .bind::<Text, _>(&hash)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let expired = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&cookie), Value::Null))
        .await
        .expect("expired response");
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(expired).await["code"], "authentication_required");
    let mut conn = app.pool.get().expect("database");
    let count = diesel::sql_query("SELECT COUNT(*) AS count FROM auth_sessions WHERE token_hash=?")
        .bind::<Text, _>(&hash)
        .get_result::<common::CountRow>(&mut conn)
        .unwrap();
    assert_eq!(count.count, 0);
}

#[tokio::test]
async fn logout_is_idempotent_and_revokes_only_current_browser() {
    let app = TestApp::spawn().await;
    let (first, _, _) = login(&app, "alice", "alice").await;
    let (second, _, _) = login(&app, "alice", "alice").await;
    let logout = app
        .router
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/logout",
            Some(&first),
            Value::Null,
        ))
        .await
        .expect("logout");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        logout.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let first_me = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&first), Value::Null))
        .await
        .unwrap();
    let second_me = app
        .router
        .clone()
        .oneshot(request("GET", "/api/me", Some(&second), Value::Null))
        .await
        .unwrap();
    assert_eq!(first_me.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(second_me.status(), StatusCode::OK);
    for cookie in [Some(first.as_str()), None] {
        let response = app
            .router
            .clone()
            .oneshot(request("POST", "/api/auth/logout", cookie, Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
