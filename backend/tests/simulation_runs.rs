mod common;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use diesel::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Notify;
use tower::ServiceExt;
use zjudss::agent::{
    FailureAnalysis, FailureAnalysisError, FailureAnalysisEvidence, FailureAnalysisRequest,
    FailureAnalyzer,
};
use zjudss::application::{CreateScriptRequest, PublishDraftRequest, ScriptSource};

#[cfg(feature = "e2e-test")]
use zjudss::testing::{EngineScenario, ScenarioEngine, ScriptedAuthor};

#[cfg(feature = "e2e-test")]
#[derive(Default)]
struct PendingFailureAnalyzer {
    entered: Notify,
}

#[cfg(feature = "e2e-test")]
#[async_trait]
impl FailureAnalyzer for PendingFailureAnalyzer {
    async fn analyze_failure(
        &self,
        _request: FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, FailureAnalysisError> {
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[cfg(feature = "e2e-test")]
impl PendingFailureAnalyzer {
    async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }
}

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
    client_id: &str,
) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/scripts/{script_id}/runs"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({"clientRunId":client_id}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_a_session_with_a_completed_run_removes_its_history_and_storage() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "delete-session-run").await;
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

    let token = common::session_deletion_token(&app, session_id, &cookie).await;
    let deleted = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{session_id}?impactToken={token}"))
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

#[tokio::test]
async fn deleting_a_run_before_its_session_does_not_release_tombstoned_artifacts_twice() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "delete-run-before-session").await;
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

    let preview = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/runs/{run_id}/deletion-preview"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview: Value =
        serde_json::from_slice(&to_bytes(preview.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let run_token = preview["impactToken"].as_str().unwrap();
    let deleted_run = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/runs/{run_id}?impactToken={run_token}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted_run.status(), StatusCode::NO_CONTENT);

    let session_token = common::session_deletion_token(&app, session_id, &cookie).await;
    let deleted_session = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!(
                "/api/sessions/{session_id}?impactToken={session_token}"
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let status = deleted_session.status();
    let body = to_bytes(deleted_session.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
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

#[tokio::test]
async fn deleting_a_session_with_repair_attempt_lineage_removes_the_session() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, _version_id, _script_id) = setup(&app).await;
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query(
        "INSERT INTO managed_workflows(
           id,session_id,status,intent_json,correction_budget,corrections_used,created_at_ms,updated_at_ms
         ) VALUES(900,?1,'failed','{}',2,2,1,1)",
    )
    .bind::<diesel::sql_types::Integer, _>(session_id)
    .execute(&mut conn)
    .unwrap();
    diesel::sql_query(
        "INSERT INTO workflow_attempts(
           id,workflow_id,attempt_number,parent_attempt_id,phase,status,agent_skill_refs_json,created_at_ms
         ) VALUES
           (901,900,1,NULL,'generation','engine_failed','[]',1),
           (902,900,2,901,'repair','engine_failed','[]',2),
           (903,900,3,902,'repair','engine_failed','[]',3)",
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);

    let token = common::session_deletion_token(&app, session_id, &cookie).await;
    let deleted = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{session_id}?impactToken={token}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = deleted.status();
    let body = to_bytes(deleted.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        app.state
            .session_repo
            .find(&user.id, session_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn failed_run_publishes_a_terminal_failed_progress_event() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"/srv/private/run.rss token=secret parse failed".to_vec(),
        exit_code: 7,
    });
    let analyzer = Arc::new(PendingFailureAnalyzer::default());
    let app = TestApp::builder()
        .engine(Arc::new(engine))
        .failure_analyzer(analyzer.clone())
        .spawn()
        .await;
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "failed-run").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            if run.status == "failed" {
                break run;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    analyzer.wait_until_entered().await;

    let stored = app.state.run_service.find(&user, run_id).await.unwrap();
    let stored_error: Value = serde_json::from_str(stored.error_json.as_deref().unwrap()).unwrap();
    assert_eq!(stored_error["analysisStatus"], "pending");
    assert_eq!(stored_error["engineLogs"][0]["stream"], "stderr");
    assert_eq!(
        stored_error["engineLogs"][0]["text"],
        "[服务器路径已隐藏] token=[敏感信息已隐藏] parse failed"
    );

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
    assert_eq!(failed_run["error"]["analysisStatus"], "pending");
    assert_eq!(failed_run["error"]["engineLogs"][0]["stream"], "stderr");
    assert_eq!(
        failed_run["error"]["engineLogs"][0]["text"],
        "[服务器路径已隐藏] token=[敏感信息已隐藏] parse failed"
    );
    assert!(failed_run["error"].get("engineOutput").is_none());
    assert!(
        failed_run["error"]["engineLogs"][0]
            .get("fullHash")
            .is_none()
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

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn failed_run_persists_structured_analysis_after_logs_and_terminal_progress() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"parse failed".to_vec(),
        exit_code: 7,
    });
    let analyzer = Arc::new(ScriptedAuthor::default());
    analyzer.enqueue_analysis(Ok(FailureAnalysis {
        summary: "脚本解析失败".into(),
        facts: vec![FailureAnalysisEvidence {
            statement: "引擎报告解析失败".into(),
            evidence: "stderr: parse failed".into(),
        }],
        possible_causes: Vec::new(),
        unknowns: vec!["具体语法位置".into()],
        recommendations: vec!["核对脚本语法".into()],
    }));
    let app = TestApp::builder()
        .engine(Arc::new(engine))
        .failure_analyzer(analyzer)
        .spawn()
        .await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "failed-run-analysis").await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();

    let stored_error = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            let error = run
                .error_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            if error
                .as_ref()
                .is_some_and(|value| value["analysisStatus"] == "complete")
            {
                break error.unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(stored_error["analysis"]["summary"], "脚本解析失败");
    assert_eq!(stored_error["engineLogs"][0]["text"], "parse failed");

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
    let public_run: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(public_run["error"]["analysisStatus"], "complete");
    assert_eq!(public_run["error"]["analysis"]["summary"], "脚本解析失败");
    assert_eq!(public_run["error"]["engineLogs"][0]["text"], "parse failed");
    assert!(
        public_run["error"]["engineLogs"][0]
            .get("fullHash")
            .is_none()
    );
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn failed_run_keeps_logs_when_failure_analysis_is_unavailable() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"solver failed".to_vec(),
        exit_code: 9,
    });
    let analyzer = Arc::new(ScriptedAuthor::default());
    analyzer.enqueue_analysis(Err("provider unavailable".into()));
    let app = TestApp::builder()
        .engine(Arc::new(engine))
        .failure_analyzer(analyzer)
        .spawn()
        .await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "failed-run-unavailable").await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();

    let stored_error = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            let error = run
                .error_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            if error
                .as_ref()
                .is_some_and(|value| value["analysisStatus"] == "unavailable")
            {
                break error.unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(stored_error["engineLogs"][0]["text"], "solver failed");
    assert!(stored_error.get("analysis").is_none());
}

#[cfg(feature = "e2e-test")]
#[tokio::test(start_paused = true)]
async fn failed_run_times_out_analysis_without_reopening_the_terminal_run() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"timeout diagnosis log".to_vec(),
        exit_code: 12,
    });
    let analyzer = Arc::new(PendingFailureAnalyzer::default());
    let app = TestApp::builder()
        .engine(Arc::new(engine))
        .failure_analyzer(analyzer.clone())
        .spawn()
        .await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "failed-run-timeout").await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();
    analyzer.wait_until_entered().await;

    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    let mut stored_error = None;
    for _ in 0..100 {
        let run = app.state.run_service.find(&user, run_id).await.unwrap();
        let error: Value = serde_json::from_str(run.error_json.as_deref().unwrap()).unwrap();
        if error["analysisStatus"] == "unavailable" {
            stored_error = Some((run.status, error));
            break;
        }
        tokio::task::yield_now().await;
    }
    let stored_error = stored_error.expect("analysis timeout must complete pending diagnosis");

    assert_eq!(stored_error.0, "failed");
    assert_eq!(
        stored_error.1["engineLogs"][0]["text"],
        "timeout diagnosis log"
    );
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn restart_recovery_resolves_pending_run_analysis_without_changing_logs() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"persistent failure".to_vec(),
        exit_code: 11,
    });
    let analyzer = Arc::new(PendingFailureAnalyzer::default());
    let app = TestApp::builder()
        .engine(Arc::new(engine))
        .failure_analyzer(analyzer.clone())
        .spawn()
        .await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "failed-run-recovery").await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();
    analyzer.wait_until_entered().await;

    let before = app.state.run_service.find(&user, run_id).await.unwrap();
    let before: Value = serde_json::from_str(before.error_json.as_deref().unwrap()).unwrap();
    let mut conn = app.pool.get().unwrap();
    zjudss::db::recover_interrupted_runs(&mut conn, 99).unwrap();
    drop(conn);
    let after = app.state.run_service.find(&user, run_id).await.unwrap();
    let after: Value = serde_json::from_str(after.error_json.as_deref().unwrap()).unwrap();

    assert_eq!(before["analysisStatus"], "pending");
    assert_eq!(after["analysisStatus"], "unavailable");
    assert_eq!(after["engineLogs"], before["engineLogs"]);
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn empty_result_run_reports_stable_error_and_can_be_deleted_atomically() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Empty);
    let app = TestApp::builder()
        .engine(std::sync::Arc::new(engine))
        .spawn()
        .await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "empty-result-run").await;
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

    let detail = app
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
    assert_eq!(detail.status(), StatusCode::OK);
    let detail: Value =
        serde_json::from_slice(&to_bytes(detail.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(detail["error"]["code"], "no_result_dataset");

    let preview = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/runs/{run_id}/deletion-preview"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview: Value =
        serde_json::from_slice(&to_bytes(preview.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(preview["resultCount"], 0);
    let token = preview["impactToken"].as_str().unwrap();
    let deleted = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/runs/{run_id}?impactToken={token}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(app.state.run_service.find(&user, run_id).await.is_err());
}

#[tokio::test]
async fn successful_run_publishes_a_terminal_done_progress_event() {
    let app = TestApp::spawn().await;
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "successful-progress").await;
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
    let response = post_run(&app, &cookie, &script_id, "click-one").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = value["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(value["run"]["modelVersionId"], version_id);
    assert_eq!(value["run"]["scriptResourceId"], script_id);
    assert_eq!(value["run"]["scriptName"], "flow.rss");
    assert!(value["run"]["scriptMessageId"].is_string());
    let retry = post_run(&app, &cookie, &script_id, "click-one").await;
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
    assert_eq!(run.script_content_hash.as_deref().unwrap().len(), 64);
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
    let second = post_run(&app, &cookie, &script_id, "click-two").await;
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
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;
    let response = post_run(&app, &cookie, &script_id, "scope-run").await;
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

#[tokio::test]
async fn manual_script_runs_once_without_an_active_model_and_freezes_its_message_anchor() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "resource-only run", 1)
        .await
        .unwrap();
    let script = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "resource-only.rss".into(),
                content: "result = source".into(),
                source: ScriptSource::Manual,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
                study_plan: None,
            },
        )
        .await
        .unwrap();

    let response = post_run(&app, &cookie, &script.id, "manual-without-model").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert!(value["run"]["modelVersionId"].is_null());
    assert!(value["run"]["scriptMessageId"].is_string());
    let run_id = value["run"]["id"].as_str().unwrap().parse().unwrap();
    let run = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run = app.state.run_service.find(&user, run_id).await.unwrap();
            if matches!(run.status.as_str(), "succeeded" | "failed") {
                break run;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run.status, "succeeded", "{:?}", run.error_json);
    assert_eq!(run.run_kind, "manual");
}
