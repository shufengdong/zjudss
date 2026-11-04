use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt;

use super::TestApp;

pub async fn session_deletion_token(app: &TestApp, session_id: i32, cookie: &str) -> String {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session_id}/deletion-preview"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["impactToken"]
        .as_str()
        .unwrap()
        .to_string()
}
