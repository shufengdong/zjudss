mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::agent::{ScriptAuthor, ScriptGenerationRequest, ScriptRepairRequest};
use zjudss::application::{
    CandidateDecision, CandidatePurpose, ConfirmStudyRequest, LockedStudyContract,
    PrepareStudyRequest, PublishDraftRequest, ReplacementAction, UpdateStudyStructRequest,
    WorkflowInputSnapshot,
};
use zjudss::db::repositories::{ManagedWorkflowRepo, ManagedWorkflowRepoError};
use zjudss::testing::{AuthorTurn, ScriptedAuthor};

use common::TestApp;

async fn ready_study(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
) -> zjudss::application::StudyConfigurationView {
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .expect("importable case");
    let batch = app
        .state
        .case_import_service
        .apply(
            user,
            session_id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            "workflow-case".into(),
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: (candidate.purpose == Some(CandidatePurpose::ModelTable))
                .then(|| candidate.proposed_alias.clone().unwrap()),
            display_name: None,
            alias: None,
        })
        .collect();
    app.state
        .data_resource_service
        .confirm_upload(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            None,
            "workflow-import",
        )
        .await
        .unwrap();
    let workspace = app
        .state
        .version_service
        .workspace(user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.unwrap().draft;
    let published = app
        .state
        .version_service
        .publish(
            user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "workflow-model".into(),
                description: Some("workflow fixture".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            user,
            published.version.version.id,
            zjudss::application::ActivateVersionRequest {
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
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn confirmation_is_idempotent_and_creates_one_provisional_run() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "managed", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let request = ConfirmStudyRequest {
        expected_revision: configuration.revision,
        client_operation_id: "confirm-1".into(),
    };

    let first = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(&user, configuration.id, request.clone())
        .await
        .unwrap();
    let replay = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(&user, configuration.id, request)
        .await
        .unwrap();

    assert_eq!(first.id, replay.id);
    assert_eq!(first.status, "queued");
    assert_eq!(first.script_message_id, None);
    assert_eq!(first.run_id, replay.run_id);
    let mut conn = app.pool.get().unwrap();
    #[derive(QueryableByName)]
    struct ProvisionalRun {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        unbound: i64,
    }
    let row = diesel::sql_query(
        "SELECT count(*) count, sum(script_resource_id IS NULL AND script_artifact_id IS NULL AND script_content_hash IS NULL) unbound FROM simulation_runs WHERE managed_workflow_id=?1",
    )
    .bind::<diesel::sql_types::Integer, _>(first.id)
    .get_result::<ProvisionalRun>(&mut conn)
    .unwrap();
    assert_eq!((row.count, row.unbound), (1, 1));
    drop(conn);

    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            first.id,
            "generation",
            None,
            "x = 1;",
            Some("test".into()),
        )
        .await
        .unwrap();
    let first_attempt = candidate.attempts[0].id;
    let repaired = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            first.id,
            "repair",
            Some(first_attempt),
            "x = 2;",
            Some("test".into()),
        )
        .await
        .unwrap();
    assert_eq!(repaired.attempts[1].parent_attempt_id, Some(first_attempt));
    assert_ne!(
        repaired.attempts[0].script_content_hash,
        repaired.attempts[1].script_content_hash
    );
    let last_attempt = repaired.attempts[1].id;
    let mut conn = app.pool.get().unwrap();
    assert!(
        diesel::sql_query(
            "UPDATE workflow_attempts SET script_content_hash=printf('%064d',9) WHERE id=?1"
        )
        .bind::<diesel::sql_types::Integer, _>(last_attempt)
        .execute(&mut conn)
        .is_err()
    );
    drop(conn);
    let terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, first.id, last_attempt, true, None)
        .await
        .unwrap();
    let replay_terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, first.id, last_attempt, true, None)
        .await
        .unwrap();
    assert_eq!(terminal.status, "succeeded");
    assert_eq!(
        terminal.script_message_id,
        replay_terminal.script_message_id
    );
    assert!(terminal.script_message_id.is_some());
    assert_eq!(
        terminal.final_script_resource_id,
        replay_terminal.final_script_resource_id
    );
}

#[tokio::test]
async fn confirmation_transaction_rejects_an_active_model_changed_after_preparation() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "active race", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap();
    assert!(prepared.replacement_actions.is_empty());
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE workspaces SET active_version_id=NULL WHERE session_id=?1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .execute(&mut conn)
        .unwrap();
    drop(conn);

    let error = ManagedWorkflowRepo::new(app.pool.clone())
        .confirm_and_start(
            &user.id,
            prepared,
            vec![],
            "active-race".into(),
            3,
            "test".into(),
            "[]".into(),
            2,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ManagedWorkflowRepoError::ConfigurationStale
    ));
}

#[tokio::test]
async fn one_live_workflow_per_session_and_cancel_is_idempotent() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "single workflow", 1)
        .await
        .unwrap();
    let first_config = ready_study(&app, &user, session.id).await;
    let first = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            first_config.id,
            ConfirmStudyRequest {
                expected_revision: first_config.revision,
                client_operation_id: "first".into(),
            },
        )
        .await
        .unwrap();
    let second_config = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"再算一次潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let error = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            second_config.id,
            ConfirmStudyRequest {
                expected_revision: second_config.revision,
                client_operation_id: "second".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "workflow_in_progress");
    assert_eq!(error.workflow_id(), Some(first.id));

    app.state
        .managed_workflow_service
        .record_candidate_script(&user, first.id, "generation", None, "x = 1;", None)
        .await
        .unwrap();
    let cancelled = app
        .state
        .managed_workflow_service
        .cancel(&user, first.id)
        .await
        .unwrap();
    let replay = app
        .state
        .managed_workflow_service
        .cancel(&user, first.id)
        .await
        .unwrap();
    assert_eq!(cancelled.status, "cancelled_by_user");
    assert_eq!(cancelled.status, replay.status);
    assert!(cancelled.final_script_resource_id.is_some());
    assert!(cancelled.script_message_id.is_some());
}

#[tokio::test]
async fn workflow_routes_are_scoped_and_return_stable_conflicts() {
    let app = TestApp::spawn().await;
    let alice = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&alice.id, "workflow api", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &alice, session.id).await;
    let cookie = app.login_cookie("alice@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/study-configurations/{}/confirm",
                configuration.id
            ))
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"expectedRevision":configuration.revision,"clientOperationId":"api-confirm"})
                    .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let workflow: Value = serde_json::from_slice(&body).unwrap();
    let id = workflow["id"].as_i64().unwrap();

    let foreign = app.login_cookie("bob@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/workflows/{id}"))
                .header("cookie", foreign)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/workflows/{id}/cancel"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn confirm_route_preserves_study_configuration_error_contracts() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "incomplete config", 1)
        .await
        .unwrap();
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/study-configurations/{}/confirm",
                configuration.id
            ))
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"expectedRevision":configuration.revision,"clientOperationId":"incomplete"})
                    .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["code"], "configuration_incomplete");
}

#[tokio::test]
async fn restart_recovery_finishes_workflow_attempt_and_provisional_run() {
    let app = TestApp::spawn().await;
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query(
        "INSERT INTO sessions(id,user_id,title,title_source,deletion_status,created_at_ms,updated_at_ms) VALUES(900,'alice@example.com','recovery','user','live',1,1)",
    )
    .execute(&mut conn)
    .unwrap();
    diesel::sql_query(
        "INSERT INTO managed_workflows(id,session_id,status,intent_json,created_at_ms,updated_at_ms) VALUES(901,900,'generating','{}',1,1)",
    )
    .execute(&mut conn)
    .unwrap();
    diesel::sql_query(
        "INSERT INTO workflow_attempts(id,workflow_id,attempt_number,phase,status,agent_skill_refs_json,created_at_ms) VALUES(902,901,1,'generation','candidate','[]',1)",
    )
    .execute(&mut conn)
    .unwrap();
    diesel::sql_query(
        "INSERT INTO simulation_runs(id,session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,managed_workflow_id,run_kind,created_at_ms) VALUES(903,900,'managed:901','queued','generating','test','[]',901,'managed',1)",
    )
    .execute(&mut conn)
    .unwrap();

    zjudss::db::recover_interrupted_runs(&mut conn, 99).unwrap();
    #[derive(QueryableByName)]
    struct Recovery {
        #[diesel(sql_type = diesel::sql_types::Text)]
        workflow_status: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        attempt_status: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        run_status: String,
    }
    let row = diesel::sql_query(
        "SELECT w.status workflow_status,a.status attempt_status,r.status run_status FROM managed_workflows w JOIN workflow_attempts a ON a.workflow_id=w.id JOIN simulation_runs r ON r.managed_workflow_id=w.id WHERE w.id=901",
    )
    .get_result::<Recovery>(&mut conn)
    .unwrap();
    assert_eq!(row.workflow_status, "service_restarted");
    assert_eq!(row.attempt_status, "engine_failed");
    assert_eq!(row.run_status, "failed");
}

#[tokio::test]
async fn restart_recovery_publishes_the_last_candidate_as_the_stable_script_anchor() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "candidate recovery", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "candidate-recovery".into(),
            },
        )
        .await
        .unwrap();
    app.state
        .managed_workflow_service
        .record_candidate_script(&user, workflow.id, "generation", None, "x = 1;", None)
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    zjudss::db::recover_interrupted_runs(&mut conn, 99).unwrap();
    drop(conn);

    let recovered = app
        .state
        .managed_workflow_service
        .get(&user, workflow.id)
        .await
        .unwrap();
    assert_eq!(recovered.status, "service_restarted");
    assert!(recovered.final_script_resource_id.is_some());
    assert!(recovered.script_message_id.is_some());
}

#[tokio::test]
async fn restart_recovery_skips_workflows_owned_by_a_deleting_session() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "deleting recovery", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "deleting-recovery".into(),
            },
        )
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE sessions SET deletion_status='deleting' WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .execute(&mut conn)
        .unwrap();
    zjudss::db::recover_interrupted_runs(&mut conn, 99).unwrap();
    #[derive(QueryableByName)]
    struct Status {
        #[diesel(sql_type = diesel::sql_types::Text)]
        status: String,
    }
    let row = diesel::sql_query("SELECT status FROM managed_workflows WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(workflow.id)
        .get_result::<Status>(&mut conn)
        .unwrap();
    assert_eq!(row.status, "queued");
}

#[tokio::test]
async fn confirmation_atomically_replaces_struct_resource_and_freezes_new_artifact() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "replace para", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    let old_id = para.binding.resource_id.clone().unwrap();
    let mut values = para.struct_card.as_ref().unwrap().values.clone();
    let previous = values["line_model"].as_str().unwrap_or("exact");
    values["line_model"] = json!(if previous.eq_ignore_ascii_case("exact") {
        "modified"
    } else {
        "exact"
    });
    let edited = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values,
                replacement_action: Some(ReplacementAction::Replace),
            },
        )
        .await
        .unwrap();
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            edited.id,
            ConfirmStudyRequest {
                expected_revision: edited.revision,
                client_operation_id: "replace-confirm".into(),
            },
        )
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    #[derive(QueryableByName)]
    struct FrozenResource {
        #[diesel(sql_type = diesel::sql_types::Text)]
        resource_id: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        old_status: String,
    }
    let frozen = diesel::sql_query(
        "SELECT w.resource_id,(SELECT status FROM data_resources WHERE id=?2) old_status FROM workflow_input_snapshots w WHERE w.workflow_id=?1 AND w.input_name='para'",
    )
    .bind::<diesel::sql_types::Integer, _>(workflow.id)
    .bind::<diesel::sql_types::Text, _>(&old_id)
    .get_result::<FrozenResource>(&mut conn)
    .unwrap();
    assert_ne!(frozen.resource_id, old_id);
    assert_eq!(frozen.old_status, "replaced");
}

#[tokio::test]
async fn concurrent_confirmation_replay_releases_unconsumed_replacement_artifact_reference() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "replacement replay", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    let mut values = para.struct_card.as_ref().unwrap().values.clone();
    let previous = values["line_model"].as_str().unwrap_or("exact");
    values["line_model"] = json!(if previous.eq_ignore_ascii_case("exact") {
        "modified"
    } else {
        "exact"
    });
    let edited = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values,
                replacement_action: Some(ReplacementAction::Replace),
            },
        )
        .await
        .unwrap();
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, edited.id, edited.revision)
        .await
        .unwrap();
    let first_materials = app
        .state
        .data_resource_service
        .materialize_study_replacements(&user, &prepared)
        .await
        .unwrap();
    let replay_materials = app
        .state
        .data_resource_service
        .materialize_study_replacements(&user, &prepared)
        .await
        .unwrap();
    let replay_artifact_ids = replay_materials
        .iter()
        .map(|material| material.artifact_id)
        .collect::<Vec<_>>();
    let repo = ManagedWorkflowRepo::new(app.pool.clone());
    let first = repo
        .confirm_and_start(
            &user.id,
            prepared.clone(),
            first_materials,
            "replacement-replay".into(),
            3,
            "test".into(),
            "[]".into(),
            2,
        )
        .await
        .unwrap();
    let replay = repo
        .confirm_and_start(
            &user.id,
            prepared,
            replay_materials,
            "replacement-replay".into(),
            3,
            "test".into(),
            "[]".into(),
            3,
        )
        .await
        .unwrap();
    assert!(first.replacements_consumed);
    assert!(!replay.replacements_consumed);
    assert_eq!(first.aggregate.workflow.id, replay.aggregate.workflow.id);
    for artifact_id in replay_artifact_ids {
        app.state
            .artifact_service
            .release(user.storage_scope_id, artifact_id)
            .await
            .unwrap();
    }

    #[derive(QueryableByName)]
    struct RefCount {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        ref_count: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        resource_refs: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        snapshot_refs: i64,
    }
    let mut conn = app.pool.get().unwrap();
    let refs = diesel::sql_query(
        "SELECT s.ref_count, \
           (SELECT count(*) FROM data_resources d WHERE d.normalized_artifact_id=s.artifact_id) resource_refs, \
           (SELECT count(*) FROM workflow_input_snapshots i WHERE i.artifact_id=s.artifact_id) snapshot_refs \
         FROM storage_scope_artifacts s \
         WHERE s.artifact_id=(SELECT artifact_id FROM workflow_input_snapshots WHERE workflow_id=?1 AND input_name='para')",
    )
    .bind::<diesel::sql_types::Integer, _>(first.aggregate.workflow.id)
    .get_result::<RefCount>(&mut conn)
    .unwrap();
    assert_eq!(refs.resource_refs, 1);
    assert_eq!(refs.snapshot_refs, 1);
    assert_eq!(refs.ref_count, refs.resource_refs + refs.snapshot_refs);
}

#[tokio::test]
async fn deleting_session_prevents_terminal_script_publication() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "delete race", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "delete-race".into(),
            },
        )
        .await
        .unwrap();
    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(&user, workflow.id, "generation", None, "x = 1;", None)
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE sessions SET deletion_status='deleting' WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let error = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, workflow.id, candidate.attempts[0].id, true, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "not_found");
    let mut conn = app.pool.get().unwrap();
    let count = diesel::sql_query(
        "SELECT count(*) count FROM script_resources WHERE managed_workflow_id=?1",
    )
    .bind::<diesel::sql_types::Integer, _>(workflow.id)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    assert_eq!(count.count, 0);
}

#[tokio::test]
async fn scripted_author_sequences_generation_and_repair() {
    let author = ScriptedAuthor::default();
    author.enqueue(AuthorTurn::Script("result = f(data, para);".into()));
    author.enqueue(AuthorTurn::Script("result = fixed(data, para);".into()));
    let locked = LockedStudyContract {
        configuration_id: 1,
        facade_id: "fixture".into(),
        function: "f".into(),
        facade_contract_hash: "hash".into(),
        engine_snapshot_id: "snapshot".into(),
        input_aliases: vec!["data".into(), "para".into()],
        structs: vec![],
        results: vec![],
    };
    let inputs = WorkflowInputSnapshot {
        model: None,
        resources: vec![],
    };
    let script = author
        .generate(ScriptGenerationRequest {
            locked: locked.clone(),
            inputs: inputs.clone(),
            engine_docs: vec![],
            instruction: "生成完整脚本".into(),
        })
        .await
        .unwrap();
    assert_eq!(script, "result = f(data, para);");
    let repaired = author
        .repair(ScriptRepairRequest {
            current_script: script,
            sanitized_stderr: "bad call".into(),
            diagnostic: None,
            locked_contract: locked,
            inputs,
            instruction: "只输出修改后的完整脚本，不要解释".into(),
        })
        .await
        .unwrap();
    assert_eq!(repaired, "result = fixed(data, para);");
    assert_eq!(author.trace(), ["generate", "repair"]);
}
