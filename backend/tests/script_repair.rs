mod common;

use std::sync::Arc;

use serde_json::json;
use zjudss::application::{
    ActivateVersionRequest, ConfirmStudyRequest, PrepareStudyRequest, PublishDraftRequest,
    StorageLimits,
};
use zjudss::testing::{
    AuthorTurn, EngineScenario, IdDatasetFixture, ScenarioEngine, ScriptedAgentFactory,
    ScriptedAuthor,
};

use common::{TestApp, publish_model_bytes};

async fn ready_minimal_study(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    name: &str,
) -> zjudss::application::StudyConfigurationView {
    let session = app
        .state
        .session_repo
        .create(&user.id, name, 1)
        .await
        .unwrap();
    let draft = publish_model_bytes(
        app,
        user,
        session.id,
        &format!("{name}-upload"),
        "table.csv",
        b"id\nrow-1\n",
        None,
        Some("table"),
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: format!("{name}-publish"),
                description: Some("managed workflow fixture".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            user,
            published.version.version.id,
            ActivateVersionRequest {
                discard_draft: false,
                expected_draft: None,
            },
        )
        .await
        .unwrap();
    app.state
        .study_configuration_service
        .prepare(
            user,
            session.id,
            PrepareStudyRequest {
                facade_id: "test".into(),
                intent: json!({"request":"run minimal study"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap()
}

async fn confirm(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    name: &str,
) -> zjudss::application::ManagedWorkflowView {
    let configuration = ready_minimal_study(app, user, name).await;
    app.state
        .managed_workflow_service
        .confirm_configuration_and_start(
            user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: format!("{name}-confirm"),
            },
        )
        .await
        .unwrap()
}

fn script_turn(script: &str) -> AuthorTurn {
    AuthorTurn::Script(script.into())
}

fn success() -> EngineScenario {
    EngineScenario::IdSuccess(vec![IdDatasetFixture {
        name: "result".into(),
        ids: vec!["row-1".into()],
    }])
}

#[tokio::test]
async fn generated_script_executes_immediately_and_publishes_exact_results() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(script_turn("result = test(table, para);"));
    engine.enqueue(success());
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "success").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(
        terminal.status,
        "succeeded",
        "{terminal:?} traces={:?}",
        author.trace()
    );
    assert_eq!(terminal.attempts.len(), 1);
    assert_eq!(terminal.attempts[0].status, "succeeded");
    assert_eq!(engine.calls().len(), 1);
    let datasets = app
        .state
        .run_service
        .datasets(&user, terminal.run_id)
        .await
        .unwrap();
    assert_eq!(datasets.len(), 1);
    assert_eq!(datasets[0].result_name, "result");
}

#[tokio::test]
async fn nonzero_exit_is_sanitized_repaired_and_then_succeeds() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(script_turn("result = test(table, para);"));
    author.enqueue(script_turn("result = test(table, para);"));
    engine.enqueue(EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"/srv/private/run/script.rss token=secret parse failed".to_vec(),
        exit_code: 7,
    });
    engine.enqueue(success());
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "repair").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "succeeded", "{terminal:?}");
    assert_eq!(terminal.corrections_used, 1);
    assert_eq!(terminal.attempts.len(), 2);
    assert_eq!(engine.calls().len(), 2);
    let prompt = serde_json::to_string(&author.repair_requests()[0]).unwrap();
    assert!(prompt.contains("只输出修改后的完整脚本，不要解释"));
    assert!(prompt.contains("服务器路径已隐藏"));
    assert!(prompt.contains("敏感信息已隐藏"));
    assert!(!prompt.contains("/srv/private"));
    assert!(!prompt.contains("token=secret"));
}

#[tokio::test]
async fn initial_contract_rejection_does_not_execute_and_repair_consumes_budget() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(script_turn(
        "x = import_dss(\"model.dss\"); result = test(table);",
    ));
    author.enqueue(script_turn("result = test(table, para);"));
    engine.enqueue(success());
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author,
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "contract").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "succeeded", "{terminal:?}");
    assert_eq!(terminal.corrections_used, 1);
    assert_eq!(terminal.attempts[0].status, "contract_rejected");
    assert_eq!(engine.calls().len(), 1);
}

#[tokio::test]
async fn timeout_is_not_repaired() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(script_turn("result = test(table, para);"));
    engine.enqueue(EngineScenario::Failure {
        code: "engine_timeout".into(),
        stderr: b"timeout".to_vec(),
        exit_code: 0,
    });
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "timeout").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "failed");
    assert_eq!(terminal.corrections_used, 0);
    assert_eq!(author.trace().len(), 1);
    assert_eq!(engine.calls().len(), 1);
}

#[tokio::test]
async fn repair_budget_allows_at_most_four_engine_calls() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    for index in 0..4 {
        author.enqueue(script_turn("result = test(table, para);"));
        engine.enqueue(EngineScenario::Failure {
            code: "engine_exit_nonzero".into(),
            stderr: format!("failure {index}").into_bytes(),
            exit_code: 7,
        });
    }
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author,
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "budget").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "failed");
    assert_eq!(terminal.corrections_used, 3);
    assert_eq!(terminal.attempts.len(), 4);
    assert_eq!(engine.calls().len(), 4);
    assert_eq!(terminal.error.unwrap()["code"], "repair_exhausted");
}

#[tokio::test]
async fn result_contract_rejection_repairs_without_publishing_partial_results() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(script_turn("result = test(table, para);"));
    author.enqueue(script_turn("result = test(table, para);"));
    engine.enqueue(EngineScenario::IdSuccess(vec![
        IdDatasetFixture {
            name: "result".into(),
            ids: vec!["row-1".into()],
        },
        IdDatasetFixture {
            name: "unexpected".into(),
            ids: vec!["row-1".into()],
        },
    ]));
    engine.enqueue(success());
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author,
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "result-contract").await;

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "succeeded", "{terminal:?}");
    assert_eq!(terminal.corrections_used, 1);
    assert_eq!(terminal.attempts[0].status, "result_rejected");
    assert_eq!(engine.calls().len(), 2);
    let datasets = app
        .state
        .run_service
        .datasets(&user, terminal.run_id)
        .await
        .unwrap();
    assert_eq!(datasets.len(), 1);
    assert_eq!(datasets[0].result_name, "result");
}

#[tokio::test]
async fn cancellation_interrupts_the_author_without_creating_a_candidate() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    author.enqueue(AuthorTurn::Wait("author-blocked".into()));
    author.enqueue(script_turn("result = test(table, para);"));
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "cancel-author").await;
    let service = app.state.managed_workflow_service.clone();
    let principal = user.clone();
    let workflow_id = workflow.id;
    let running = tokio::spawn(async move {
        service
            .execute_confirmed(&principal, workflow_id)
            .await
            .unwrap();
    });
    assert!(author.wait_until_blocked("author-blocked").await);

    let cancelled = app
        .state
        .managed_workflow_service
        .cancel(&user, workflow.id)
        .await
        .unwrap();
    running.await.unwrap();

    assert_eq!(cancelled.status, "cancelled_by_user");
    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "cancelled_by_user");
    assert!(terminal.attempts.is_empty());
    assert!(engine.calls().is_empty());
}

#[tokio::test]
async fn candidate_quota_failure_terminalizes_with_stable_code() {
    let author = Arc::new(ScriptedAuthor::default());
    author.enqueue(script_turn("result = test(table, para);"));
    let app = TestApp::spawn_minimal_with_engine_and_agent_factory(
        Arc::new(ScenarioEngine::default()),
        Arc::new(ScriptedAgentFactory::default()),
        author,
    )
    .await;
    let user = common::test_user("alice@example.com");
    let workflow = confirm(&app, &user, "candidate-quota").await;
    let usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            user.storage_scope_id,
            StorageLimits {
                artifact_limit: usage.used_artifacts,
                byte_limit: usage.byte_limit,
                warning_ratio: usage.warning_ratio,
            },
        )
        .await
        .unwrap();

    app.state
        .managed_workflow_service
        .execute_confirmed(&user, workflow.id)
        .await
        .unwrap();

    let terminal = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(terminal.status, "failed");
    assert_eq!(terminal.error.unwrap()["code"], "storage_quota_exceeded");
    assert!(terminal.attempts.is_empty());
}
