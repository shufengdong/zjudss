#![cfg(feature = "e2e-test")]

mod common;

use std::process::Stdio;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use backend::testing::{
    ControlState, E2eConfig, ScenarioEngine, ScriptedAgentFactory, control_router,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tower::ServiceExt;

use common::TestApp;

#[tokio::test]
async fn e2e_server_uses_temp_db_and_data_dir() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("isolated-e2e");
    let binary = std::env::var("CARGO_BIN_EXE_e2e-server").expect("feature-gated e2e binary");
    let mut child = tokio::process::Command::new(binary)
        .env("E2E_ROOT", &root)
        .env("E2E_BIND", "127.0.0.1:0")
        .env("E2E_CONTROL_TOKEN", "control-secret")
        .env("E2E_PROXY_TOKEN", "proxy-secret")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .expect("server readiness timeout")
    .unwrap();
    let ready = match line.strip_prefix("E2E_READY ") {
        Some(ready) => ready,
        None => {
            let status = child.wait().await.unwrap();
            let mut stderr = String::new();
            BufReader::new(child.stderr.take().unwrap())
                .read_to_string(&mut stderr)
                .await
                .unwrap();
            panic!("missing readiness line; status={status}, stdout={line:?}, stderr={stderr}");
        }
    };
    let ready: serde_json::Value = serde_json::from_str(ready).unwrap();
    assert!(ready["address"].as_str().unwrap().starts_with("127.0.0.1:"));
    assert_eq!(ready["root"], root.display().to_string());
    assert!(root.join("e2e.db").is_file());
    assert!(root.join("data").is_dir());

    child.kill().await.unwrap();
    child.wait().await.unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(!root.exists());
}

#[tokio::test]
async fn control_routes_require_token() {
    let router = control_router(ControlState::new(
        "control-secret",
        Arc::new(ScriptedAgentFactory::default()),
        Arc::new(ScenarioEngine::default()),
    ));
    for (method, uri) in [
        ("GET", "/__e2e/ready"),
        ("GET", "/__e2e/trace"),
        ("POST", "/__e2e/llm-scenarios"),
        ("POST", "/__e2e/engine-scenarios"),
        ("POST", "/__e2e/barriers/test/release"),
    ] {
        for token in [None, Some("wrong")] {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json");
            if let Some(token) = token {
                builder = builder.header("x-zjudss-e2e-token", token);
            }
            let response = router
                .clone()
                .oneshot(builder.body(Body::from("{}")).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
    }
    let response = router
        .oneshot(
            Request::builder()
                .uri("/__e2e/ready")
                .header("x-zjudss-e2e-token", "control-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn control_routes_enqueue_scenarios_and_release_known_barriers() {
    let router = control_router(ControlState::new(
        "control-secret",
        Arc::new(ScriptedAgentFactory::default()),
        Arc::new(ScenarioEngine::default()),
    ));
    for (uri, body) in [
        (
            "/__e2e/llm-scenarios",
            serde_json::json!({
                "id": "held",
                "expectedPrompt": null,
                "turns": [
                    {"kind": "delay_until_released", "barrier_id": "known"},
                    {"kind": "text", "text": "done"}
                ]
            }),
        ),
        (
            "/__e2e/engine-scenarios",
            serde_json::json!({"kind": "run_failure"}),
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("x-zjudss-e2e-token", "control-secret")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED, "{uri}");
    }
    let release = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/__e2e/barriers/known/release")
                .header("x-zjudss-e2e-token", "control-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(release.status(), StatusCode::OK);
    let trace = router
        .oneshot(
            Request::builder()
                .uri("/__e2e/trace")
                .header("x-zjudss-e2e-token", "control-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(trace.status(), StatusCode::OK);
}

#[tokio::test]
async fn control_routes_are_absent_from_production_router() {
    let app = TestApp::spawn().await;
    let response = app
        .router
        .oneshot(
            Request::builder()
                .uri("/__e2e/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn server_rejects_non_loopback_bind() {
    let root = tempfile::tempdir().unwrap();
    let error = E2eConfig::from_values(
        root.path().to_path_buf(),
        "0.0.0.0:8080",
        "control-secret",
        "proxy-secret",
    )
    .unwrap_err();
    assert_eq!(error.code(), "e2e_bind_not_loopback");
}
