mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::{CreateScriptRequest, PublishDraftRequest, ScriptSource};

#[cfg(feature = "e2e-test")]
use zjudss::testing::{EngineScenario, ScenarioEngine};

async fn setup(app: &TestApp) -> (zjudss::auth::CurrentUser, String, i32, String, String) {
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "runs", 1)
        .await
        .unwrap();
    let draft = common::publish_model_bytes(
        app,
        &user,
        session.id,
        "run-model",
        "bus.csv",
        b"id,name\n1,bus\n",
        None,
        Some("bus"),
    )
    .await;
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
                description: None,
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
                study_plan: None,
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

#[tokio::test]
async fn deleting_a_session_with_a_completed_run_removes_its_history_and_storage() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, &version_id, "delete-session-run").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            if matches!(run.status.as_str(), "succeeded" | "failed") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        app.state
            .artifact_service
            .get_usage(user.storage_scope_id)
            .await
            .unwrap()
            .used_artifacts
            > 0
    );

    let deleted = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{session_id}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let deleted_status = deleted.status();
    let deleted_body = to_bytes(deleted.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(
        deleted_status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&deleted_body)
    );
    assert!(
        app.state
            .session_repo
            .find(&user.id, session_id)
            .await
            .unwrap()
            .is_none()
    );
    let usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert_eq!(usage.used_artifacts, 0);
    assert_eq!(usage.used_bytes, 0);
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

    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/runs/{run_id}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let failed_run: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(failed_run["stage"], "run");
    assert_eq!(failed_run["error"]["code"], "run_failed");
    assert_eq!(failed_run["error"]["message"], "仿真引擎执行失败");
    assert_eq!(
        failed_run["error"]["engineOutput"],
        "RSS parse error: unexpected token\nat [服务器路径已隐藏]"
    );
    assert_eq!(failed_run["exitCode"], 7);
    assert!(!failed_run.to_string().contains("/srv/private"));

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
async fn successful_run_publishes_a_terminal_done_progress_event() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, version_id, script_id) = setup(&app).await;
    let response = post_run(
        &app,
        &cookie,
        &script_id,
        &version_id,
        "successful-progress",
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();

    let progress = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let messages = app
                .state
                .message_repo
                .list_by_session(&user.id, session_id)
                .await
                .unwrap();
            let progress = messages
                .iter()
                .filter(|message| message.run_id == Some(run_id) && message.kind == "progress")
                .map(|message| {
                    serde_json::from_str::<Value>(message.payload_json.as_deref().unwrap()).unwrap()
                })
                .collect::<Vec<_>>();
            if progress.len() == 3 {
                break progress;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert!(
        progress.last().unwrap()["stages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|stage| stage["status"] == "done")
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
    assert_eq!(value["run"]["scriptName"], "flow.rss");
    let retry = post_run(&app, &cookie, &script_id, &version_id, "click-one").await;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(retry["run"]["id"], run_id);
    assert_eq!(retry["replayed"], true);
    assert_eq!(retry["run"]["scriptName"], "flow.rss");
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
    assert_eq!(run.model_content_hash.as_deref().map(str::len), Some(64));
    assert_eq!(run.script_content_hash.len(), 64);
    for path in [
        format!("/api/sessions/{session_id}/runs"),
        format!("/api/runs/{run_id}"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(path)
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let run = body
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|runs| runs.first())
            .unwrap_or(&body);
        assert_eq!(run["scriptName"], "flow.rss");
    }
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
