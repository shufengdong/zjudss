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
use zjudss::application::RunScheduler;
use zjudss::application::{
    CreateScriptRequest, EventWriter, OperationKind, OperationRef, PublishDraftRequest,
    ScriptSource, Stage, StageStatus,
};

#[cfg(feature = "e2e-test")]
use zjudss::testing::{
    EngineScenario, IdDatasetFixture, RunControlGate, ScenarioEngine, ScriptedAuthor,
};

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

async fn post_cancel(app: &TestApp, cookie: &str, run_id: i32) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/runs/{run_id}/cancel"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn run_id_by_client(app: &TestApp, client_run_id: &str) -> i32 {
    zjudss::db::schema::simulation_runs::table
        .filter(zjudss::db::schema::simulation_runs::client_run_id.eq(client_run_id))
        .select(zjudss::db::schema::simulation_runs::id)
        .first(&mut app.pool.get().unwrap())
        .unwrap()
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn active_and_queued_runs_cancel_idempotently_without_failure_analysis_or_results() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Cancelled {
        barrier_id: "active-run".into(),
    });
    let analyzer = Arc::new(ScriptedAuthor::default());
    let app = TestApp::builder()
        .engine(Arc::new(engine.clone()))
        .failure_analyzer(analyzer.clone())
        .scheduler(RunScheduler::new(1, 1, 4))
        .spawn()
        .await;
    let (user, cookie, session_id, _version_id, script_id) = setup(&app).await;

    let first = post_run(&app, &cookie, &script_id, "cancel-active").await;
    let first: Value =
        serde_json::from_slice(&to_bytes(first.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let first_id = first["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();
    assert!(engine.wait_until_blocked("active-run").await);

    let second = post_run(&app, &cookie, &script_id, "cancel-queued").await;
    let second: Value =
        serde_json::from_slice(&to_bytes(second.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let second_id = second["run"]["id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();

    let queued_cancel = post_cancel(&app, &cookie, second_id).await;
    assert_eq!(queued_cancel.status(), StatusCode::OK);
    let queued: Value = serde_json::from_slice(
        &to_bytes(queued_cancel.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(queued["status"], "failed");
    assert_eq!(queued["stage"], "cancelled_by_user");
    assert_eq!(queued["error"]["code"], "cancelled_by_user");
    assert_eq!(engine.calls().len(), 1, "排队取消不得启动第二次引擎调用");

    let mut events = app.state.events.subscribe(session_id);
    let active_cancel = post_cancel(&app, &cookie, first_id).await;
    assert_eq!(active_cancel.status(), StatusCode::OK);
    let active: Value = serde_json::from_slice(
        &to_bytes(active_cancel.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(active["error"]["code"], "cancelled_by_user");
    let first_id_string = first_id.to_string();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            let value = serde_json::to_value(event).unwrap();
            if value["type"] == "progress"
                && value["operation"]["id"].as_str() == Some(first_id_string.as_str())
                && value["stages"][0]["key"] == "cancelled_by_user"
            {
                break value;
            }
        }
    })
    .await
    .expect("取消终态必须实时广播");
    assert_eq!(event["stages"][0]["label"], "已取消");

    let writer = EventWriter::new(
        user.id.clone(),
        session_id,
        app.state.message_repo.clone(),
        app.state.events.clone(),
    );
    writer
        .progress(
            OperationRef {
                kind: OperationKind::Run,
                id: first_id.to_string(),
            },
            vec![Stage {
                key: "run".into(),
                label: "延迟执行更新".into(),
                status: StageStatus::Running,
            }],
        )
        .await
        .unwrap();

    let repeated = post_cancel(&app, &cookie, first_id).await;
    assert_eq!(repeated.status(), StatusCode::OK);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.state.scheduler.active_count() != 0 || app.state.scheduler.waiting_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("取消释放 scheduler 名额");
    assert!(analyzer.trace().is_empty(), "用户取消不得触发失败分析");
    assert!(
        app.state
            .dataset_service
            .list(&user, first_id)
            .await
            .unwrap()
            .is_empty()
    );
    let messages = app
        .state
        .message_repo
        .list_by_session(&user.id, session_id)
        .await
        .unwrap();
    let progress = messages
        .iter()
        .filter(|message| message.run_id == Some(first_id) && message.kind == "progress")
        .collect::<Vec<_>>();
    assert_eq!(progress.len(), 1, "一个 run 只保留一条可修订进度消息");
    let payload: Value =
        serde_json::from_str(progress[0].payload_json.as_deref().unwrap()).unwrap();
    assert_eq!(payload["stages"][0]["key"], "cancelled_by_user");
    assert_eq!(payload["stages"][0]["label"], "已取消");
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn cancellation_before_signal_registration_releases_only_the_queued_ticket() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Cancelled {
        barrier_id: "registration-active".into(),
    });
    let scheduler = RunScheduler::new(1, 1, 4);
    let app = TestApp::builder()
        .engine(Arc::new(engine.clone()))
        .scheduler(scheduler.clone())
        .spawn()
        .await;
    let (_user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let active = post_run(&app, &cookie, &script_id, "registration-active").await;
    let active: Value =
        serde_json::from_slice(&to_bytes(active.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    let active_id = active["run"]["id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert!(engine.wait_until_blocked("registration-active").await);

    let gate = RunControlGate::before_registration(&app.data_dir, "cancel-before-registration");
    let router = app.router.clone();
    let queued_cookie = cookie.clone();
    let queued_script = script_id.clone();
    let submission = tokio::spawn(async move {
        router
            .oneshot(
                Request::post(format!("/api/scripts/{queued_script}/runs"))
                    .header("cookie", queued_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"clientRunId":"cancel-before-registration"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    gate.wait_until_entered().await;
    assert_eq!(scheduler.active_count(), 1);
    assert_eq!(scheduler.waiting_count(), 1);
    let queued_id = run_id_by_client(&app, "cancel-before-registration");
    assert_eq!(
        post_cancel(&app, &cookie, queued_id).await.status(),
        StatusCode::OK
    );
    gate.release();
    assert_eq!(submission.await.unwrap().status(), StatusCode::ACCEPTED);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while scheduler.waiting_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("已持久化取消必须在进入 scheduler 前释放排队名额");
    assert_eq!(scheduler.active_count(), 1, "不得靠释放活动运行来腾出队列");
    assert_eq!(engine.calls().len(), 1);
    assert_eq!(
        post_cancel(&app, &cookie, active_id).await.status(),
        StatusCode::OK
    );
}

#[cfg(feature = "e2e-test")]
#[tokio::test]
async fn cancellation_during_result_publication_finishes_cleanup_and_preserves_the_winner() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::IdSuccess(vec![IdDatasetFixture {
        name: "result".into(),
        ids: vec!["1".into()],
    }]));
    engine.enqueue(EngineScenario::IdSuccess(vec![IdDatasetFixture {
        name: "result".into(),
        ids: vec!["2".into()],
    }]));
    let app = TestApp::builder().engine(Arc::new(engine)).spawn().await;
    let (user, cookie, _session_id, _version_id, script_id) = setup(&app).await;
    let usage_before = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let objects_before = object_file_count(&app.data_dir);
    let registration = RunControlGate::before_registration(&app.data_dir, "publication-cancel");
    let router = app.router.clone();
    let run_cookie = cookie.clone();
    let run_script = script_id.clone();
    let submission = tokio::spawn(async move {
        router
            .oneshot(
                Request::post(format!("/api/scripts/{run_script}/runs"))
                    .header("cookie", run_cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"clientRunId":"publication-cancel"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    registration.wait_until_entered().await;
    let run_id = run_id_by_client(&app, "publication-cancel");
    let publication = RunControlGate::result_publication(&app.data_dir, run_id);
    registration.release();
    assert_eq!(submission.await.unwrap().status(), StatusCode::ACCEPTED);
    publication.wait_until_entered().await;
    assert_eq!(
        post_cancel(&app, &cookie, run_id).await.status(),
        StatusCode::OK
    );
    publication.release();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        publication.wait_until_finished(),
    )
    .await
    .expect("取消不得丢弃已开始的 artifact 发布与回滚清理");

    assert_eq!(
        app.state
            .run_service
            .find(&user, run_id)
            .await
            .unwrap()
            .stage,
        "cancelled_by_user"
    );
    assert!(
        app.state
            .run_service
            .datasets(&user, run_id)
            .await
            .unwrap()
            .is_empty()
    );
    let usage_after = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert_eq!(usage_after.used_artifacts, usage_before.used_artifacts);
    assert_eq!(usage_after.used_bytes, usage_before.used_bytes);
    assert_eq!(object_file_count(&app.data_dir), objects_before);

    let completed = post_run(&app, &cookie, &script_id, "publication-completes").await;
    let completed: Value =
        serde_json::from_slice(&to_bytes(completed.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let completed_id = completed["run"]["id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if app
                .state
                .run_service
                .find(&user, completed_id)
                .await
                .unwrap()
                .status
                == "succeeded"
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let terminal = post_cancel(&app, &cookie, completed_id).await;
    let terminal: Value =
        serde_json::from_slice(&to_bytes(terminal.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(terminal["status"], "succeeded");
    assert_eq!(
        app.state
            .run_service
            .datasets(&user, completed_id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(object_file_count(&app.data_dir), objects_before + 1);
}

fn object_file_count(data_dir: &std::path::Path) -> usize {
    fn count(path: &std::path::Path) -> usize {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                            count(&entry.path())
                        } else {
                            usize::from(entry.file_type().is_ok_and(|kind| kind.is_file()))
                        }
                    })
                    .sum()
            })
            .unwrap_or_default()
    }
    count(&data_dir.join("objects"))
}

#[tokio::test]
async fn managed_run_cancel_route_requires_the_workflow_endpoint() {
    let app = TestApp::spawn().await;
    let (_user, cookie, session_id, _version_id, _script_id) = setup(&app).await;
    let run_id = {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query(
            "INSERT INTO managed_workflows(session_id,status,intent_json,created_at_ms,updated_at_ms) VALUES(?1,'executing','{}',1,1)",
        )
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .execute(&mut conn)
        .unwrap();
        let workflow_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
            "last_insert_rowid()",
        ))
        .get_result::<i32>(&mut conn)
        .unwrap();
        diesel::sql_query(
            "INSERT INTO simulation_runs(session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,managed_workflow_id,run_kind,input_snapshot_json,created_at_ms) VALUES(?1,'managed-cancel-route','running','run','test','[]',?2,'managed','{}',1)",
        )
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .bind::<diesel::sql_types::Integer, _>(workflow_id)
        .execute(&mut conn)
        .unwrap();
        diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
            "last_insert_rowid()",
        ))
        .get_result::<i32>(&mut conn)
        .unwrap()
    };

    let response = post_cancel(&app, &cookie, run_id).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["code"], "managed_run_cancel_via_workflow");
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
    assert_eq!(progress.len(), 1);
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
            if progress.len() == 1
                && progress[0]["stages"]
                    .as_array()
                    .is_some_and(|stages| stages.iter().all(|stage| stage["status"] == "done"))
            {
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
    let foreign_replay = post_run(&app, &foreign, &script_id, "scope-run").await;
    assert_eq!(foreign_replay.status(), StatusCode::NOT_FOUND);
    let owner_replay = post_run(&app, &cookie, &script_id, "scope-run").await;
    assert_eq!(owner_replay.status(), StatusCode::OK);
    let owner_replay: Value = serde_json::from_slice(
        &to_bytes(owner_replay.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(owner_replay["run"]["id"], run_id);
    assert_eq!(owner_replay["replayed"], true);
    let _ = user;
}

#[tokio::test]
async fn manual_script_runs_once_without_an_active_model_and_freezes_its_message_anchor() {
    let engine = Arc::new(ScenarioEngine::default());
    engine.enqueue(EngineScenario::IdSuccess(vec![
        IdDatasetFixture {
            name: "result".into(),
            ids: vec!["row-1".into()],
        },
        IdDatasetFixture {
            name: "manual_extra".into(),
            ids: vec!["row-1".into()],
        },
    ]));
    let app = TestApp::builder().engine(engine).spawn().await;
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
    let datasets = app.state.run_service.datasets(&user, run.id).await.unwrap();
    assert_eq!(datasets.len(), 2);
    assert_eq!(datasets[0].result_name, "manual_extra");
    assert_eq!(datasets[1].result_name, "result");
}
