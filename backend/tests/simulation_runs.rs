mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{
    CreateScriptRequest, PublishDraftRequest, ScriptSource, UploadBatch, UploadFile,
};
use backend::storage::StageLimits;
use common::TestApp;
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;

#[cfg(feature = "e2e-test")]
use backend::testing::{EngineScenario, ScenarioEngine};

async fn setup(app: &TestApp) -> (backend::auth::CurrentUser, String, i32, String, String) {
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "runs", 1)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,name\n1,bus\n"), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session.id,
            "run-model".into(),
            None,
            UploadBatch {
                client_batch_id: "unused".into(),
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
        .process(&user, accepted.id.parse().unwrap())
        .await
        .unwrap();
    let draft = app
        .state
        .model_import_service
        .resolve_and_apply(
            &user,
            accepted.id.parse().unwrap(),
            None,
            ready
                .candidates
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
        )
        .await
        .unwrap();
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "run-version".into(),
            },
        )
        .await
        .unwrap();
    let script = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "flow.rss".into(),
                content: "result = bus".into(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await
        .unwrap();
    (
        user,
        cookie,
        session.id,
        published.version.version.id.to_string(),
        script.id,
    )
}

async fn post_run(
    app: &TestApp,
    cookie: &str,
    script_id: &str,
    version_id: &str,
    client_id: &str,
) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/scripts/{script_id}/runs"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"modelVersionId":version_id,"clientRunId":client_id}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn failed_run_publishes_a_terminal_failed_progress_event() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::RunFailure);
    let app = TestApp::spawn_with_engine(std::sync::Arc::new(engine)).await;
    let (user, cookie, session_id, version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, &version_id, "failed-run").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            if run.status == "failed" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let progress = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let messages = app
                .state
                .message_repo
                .list_by_session(&user.id, session_id)
                .await
                .unwrap();
            let progress: Vec<Value> = messages
                .iter()
                .filter(|message| message.run_id == Some(run_id) && message.kind == "progress")
                .map(|message| {
                    serde_json::from_str(message.payload_json.as_deref().unwrap()).unwrap()
                })
                .collect();
            if progress.last().is_some_and(|payload| {
                payload["stages"]
                    .as_array()
                    .is_some_and(|stages| stages.iter().any(|stage| stage["status"] == "failed"))
            }) {
                break progress;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(progress.len(), 3);
    assert_eq!(
        progress.last().unwrap()["operation"]["id"],
        run_id.to_string()
    );
    assert!(
        progress.last().unwrap()["stages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stage| stage["key"] == "run" && stage["status"] == "failed")
    );
}

#[tokio::test]
async fn only_explicit_click_route_creates_idempotent_frozen_runs_and_results() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, version_id, script_id) = setup(&app).await;
    assert!(
        app.state
            .run_service
            .list(&user, session_id, 50)
            .await
            .unwrap()
            .is_empty(),
        "creating a script must never auto-run"
    );
    let mismatch = post_run(&app, &cookie, &script_id, "999999", "race").await;
    assert_eq!(mismatch.status(), StatusCode::CONFLICT);
    assert!(
        app.state
            .run_service
            .list(&user, session_id, 50)
            .await
            .unwrap()
            .is_empty()
    );
    let response = post_run(&app, &cookie, &script_id, &version_id, "click-one").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(value["run"]["modelVersionId"], version_id);
    assert_eq!(value["run"]["scriptResourceId"], script_id);
    let retry = post_run(&app, &cookie, &script_id, &version_id, "click-one").await;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(retry["run"]["id"], run_id);
    assert_eq!(retry["replayed"], true);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app
                .state
                .run_service
                .find(&user, run_id.parse().unwrap())
                .await
                .unwrap();
            if matches!(run.status.as_str(), "succeeded" | "failed") {
                break run;
            }
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    let run = app
        .state
        .run_service
        .find(&user, run_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(run.status, "succeeded", "{:?}", run.error_json);
    assert_eq!(run.model_content_hash.len(), 64);
    assert_eq!(run.script_content_hash.len(), 64);
    assert_eq!(
        app.state
            .run_service
            .datasets(&user, run.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let payload = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let messages = app
                .state
                .message_repo
                .list_by_session(&user.id, session_id)
                .await
                .unwrap();
            if let Some(result) = messages.iter().find(|message| message.kind == "result") {
                break serde_json::from_str::<Value>(result.payload_json.as_deref().unwrap())
                    .unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(payload["runId"], run_id);
    assert!(payload.get("taskId").is_none());
    let second = post_run(&app, &cookie, &script_id, &version_id, "click-two").await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second: Value =
        serde_json::from_slice(&to_bytes(second.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_ne!(second["run"]["id"], run_id);
    let old = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/tasks/{run_id}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn runs_and_result_lists_are_session_and_user_scoped() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, &version_id, "scope-run").await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap();
    let foreign = app.login_cookie("bob@example.com").await;
    for path in [
        format!("/api/sessions/{session_id}/runs"),
        format!("/api/runs/{run_id}"),
        format!("/api/runs/{run_id}/datasets"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(path)
                    .header("cookie", &foreign)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let _ = user;
}
