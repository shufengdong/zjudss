mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use serde_json::Value;
use tower::ServiceExt;
use zjudss::application::{CreateScriptRequest, ScriptSource};

#[tokio::test]
async fn scripts_are_immutable_independent_validated_and_lazy_readable() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "scripts", 1)
        .await
        .unwrap();
    let long = (0..30)
        .map(|index| format!("output_{index} = bus"))
        .collect::<Vec<_>>()
        .join("\n");
    let first = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "study.rss".into(),
                content: long.clone(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await
        .unwrap();
    let second = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "copy.rss".into(),
                content: long.clone(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.validation_status, "valid");
    assert!(first.preview_truncated);
    assert_eq!(first.preview.lines().count(), 20);
    assert!(!first.can_run);
    assert_eq!(first.disabled_reason.as_deref(), Some("no_active_model"));

    let detail = app
        .state
        .script_service
        .detail(&user, first.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(detail.content, long);
    let invalid = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "unsafe.rss".into(),
                content: "x = read(\"/tmp/source\")".into(),
                source: ScriptSource::Agent,
                agent_bundle_version: Some("test".into()),
                agent_skill_refs: vec![serde_json::json!({"id":"rss-generation"})],
            },
        )
        .await
        .unwrap();
    assert_eq!(invalid.validation_status, "invalid");
    assert_eq!(
        app.state
            .script_service
            .detail(&user, invalid.id.parse().unwrap())
            .await
            .unwrap()
            .content,
        "x = read(\"/tmp/source\")"
    );

    let messages = app
        .state
        .message_repo
        .list_by_session(&user.id, session.id)
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.kind == "script")
            .count(),
        3
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.kind == "script")
            .all(|message| {
                let payload: Value =
                    serde_json::from_str(message.payload_json.as_deref().unwrap_or("{}")).unwrap();
                !payload["script"]["preview"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("output_29")
            })
    );
}

#[tokio::test]
async fn script_routes_accept_only_single_rss_and_have_no_edit_or_delete() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "upload script", 1)
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let boundary = "script-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"run.rss\"\r\nContent-Type: text/plain\r\n\r\nresult = bus\r\n--{boundary}--\r\n"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/scripts", session.id))
                .header("cookie", &cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let id = body["script"]["id"].as_str().unwrap();
    let detail = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/scripts/{id}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail: Value =
        serde_json::from_slice(&to_bytes(detail.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(detail["script"]["content"], "result = bus");
    for method in ["PATCH", "DELETE"] {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/scripts/{id}"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.router.clone().oneshot(request).await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let foreign = app.login_cookie("bob@example.com").await;
    assert_eq!(
        app.router
            .clone()
            .oneshot(
                Request::get(format!("/api/scripts/{id}"))
                    .header("cookie", foreign)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn script_text_endpoint_creates_a_manual_source_script() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "paste script", 1)
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/scripts/text", session.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"manual.rss","content":"result = bus"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["script"]["sourceKind"], "manual");
    assert_eq!(body["script"]["validationStatus"], "valid");
    assert_eq!(body["script"]["name"], "manual.rss");
}

#[tokio::test]
async fn script_resources_disappear_only_with_the_whole_session() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "cleanup", 1)
        .await
        .unwrap();
    let script = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "cleanup.rss".into(),
                content: "result = 1".into(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{}", session.id))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        app.state
            .script_service
            .detail(&user, script.id.parse().unwrap())
            .await
            .is_err()
    );
}
