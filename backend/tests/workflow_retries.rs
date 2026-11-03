mod common;

use std::sync::Arc;

use arrow_array::{Array, StringArray, UInt32Array};
use diesel::prelude::*;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};
use zjudss::application::{
    ActivateVersionRequest, CandidateDecision, CandidatePurpose, ConfirmRetryRequest,
    ConfirmStudyRequest, PrepareStudyRequest, PublishDraftRequest, RetryStudyRequest,
};
use zjudss::testing::{
    AuthorTurn, EngineScenario, IdDatasetFixture, ScenarioEngine, ScriptedAgentFactory,
    ScriptedAuthor,
};

async fn confirmed_workflow(
    app: &common::TestApp,
    user: &zjudss::auth::CurrentUser,
) -> zjudss::application::ManagedWorkflowView {
    let session = app
        .state
        .session_repo
        .create(&user.id, "retry", 1)
        .await
        .unwrap();
    let draft = common::publish_model_bytes(
        app,
        user,
        session.id,
        "retry-upload",
        "table.csv",
        b"id\nrow-1\n",
        None,
        Some("table"),
    )
    .await;
    let version = app
        .state
        .version_service
        .publish(
            user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "retry-publish".into(),
                description: Some("retry fixture".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            user,
            version.version.version.id,
            ActivateVersionRequest {
                discard_draft: false,
                expected_draft: None,
            },
        )
        .await
        .unwrap();
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            user,
            session.id,
            PrepareStudyRequest {
                facade_id: "test".into(),
                intent: json!({"request":"retry study"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    app.state
        .managed_workflow_service
        .confirm_configuration_and_start(
            user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "retry-source".into(),
            },
        )
        .await
        .unwrap()
}

fn failure() -> EngineScenario {
    EngineScenario::Failure {
        code: "engine_exit_nonzero".into(),
        stderr: b"invalid script".to_vec(),
        exit_code: 7,
    }
}

async fn failed_source(
    app: &common::TestApp,
    user: &zjudss::auth::CurrentUser,
    author: &ScriptedAuthor,
    engine: &ScenarioEngine,
) -> zjudss::application::ManagedWorkflowView {
    for _ in 0..4 {
        author.enqueue(AuthorTurn::Script("result = test(table, para);".into()));
        engine.enqueue(failure());
    }
    let source = confirmed_workflow(app, user).await;
    app.state
        .managed_workflow_service
        .execute_confirmed(user, source.id)
        .await
        .unwrap();
    app.state
        .managed_workflow_service
        .get(user, source.id)
        .await
        .unwrap()
}

async fn failed_overlay_source(
    app: &common::TestApp,
    user: &zjudss::auth::CurrentUser,
) -> zjudss::application::ManagedWorkflowView {
    let session = app
        .state
        .session_repo
        .create(&user.id, "overlay retry", 1)
        .await
        .unwrap();
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let batch = app
        .state
        .case_import_service
        .apply(
            user,
            session.id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            "overlay-retry-case".into(),
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
            "overlay-retry-import",
        )
        .await
        .unwrap();
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"retry overlay study"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "overlay-retry-source".into(),
            },
        )
        .await
        .unwrap();
    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            user,
            workflow.id,
            "generation",
            None,
            "bus_voltage = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);",
            Some("test".into()),
        )
        .await
        .unwrap();
    app.state
        .managed_workflow_service
        .finalize_with_script(
            user,
            workflow.id,
            candidate.attempts[0].id,
            false,
            Some(json!({"code":"test_failure"})),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn unchanged_private_overlay_retry_reuses_the_effective_artifact() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user).await;
    let source_script_id = source.final_script_resource_id.unwrap();
    let source_artifact_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();

    let preview = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source_script_id,
            RetryStudyRequest {
                client_retry_id: "retry-unchanged-overlay".into(),
            },
        )
        .await
        .unwrap();
    assert!(preview.retry_preview_id.is_none());
    let retry = preview.workflow.unwrap();
    let retry_artifact_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(retry.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(retry_artifact_id, source_artifact_id);
}

#[tokio::test]
async fn changed_private_overlay_retry_combines_latest_parameters_with_frozen_results() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user).await;
    let source_script_id = source.final_script_resource_id.unwrap();
    let session_id = app
        .state
        .run_service
        .find(&user, source.run_id)
        .await
        .unwrap()
        .session_id;
    let para = app
        .state
        .data_resource_service
        .list(&user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(&user, &para.id, Some(2), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    values.insert("max_iter".into(), json!(77));
    let replacement = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &para.id,
            &para.content_hash,
            "frames.para",
            Value::Object(values),
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .confirm_structured_replacement(
            &user,
            &replacement.preview_token,
            zjudss::application::ReplacementAction::Replace,
        )
        .await
        .unwrap();

    let preview = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source_script_id,
            RetryStudyRequest {
                client_retry_id: "retry-changed-overlay".into(),
            },
        )
        .await
        .unwrap();
    assert!(preview.workflow.is_none());
    assert!(
        preview
            .changes
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    let retry = app
        .state
        .managed_workflow_service
        .confirm_retry(
            &user,
            &preview.retry_preview_id.unwrap(),
            ConfirmRetryRequest { confirmed: true },
        )
        .await
        .unwrap();
    let artifact_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(retry.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    let file = app
        .state
        .artifact_service
        .open_owned(user.storage_scope_id, artifact_id)
        .await
        .unwrap();
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    let batch = reader.next().unwrap().unwrap();
    let result = batch
        .column(batch.schema().index_of("result").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(!result.is_null(0));
    assert_eq!(result.value(0), "bus_voltage");
    let max_iter = batch
        .column(batch.schema().index_of("max_iter").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(max_iter.value(0), 77);
}

#[tokio::test]
async fn unchanged_failed_agent_script_starts_a_derived_retry_with_a_fresh_budget() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    let app = common::TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let source = failed_source(&app, &user, &author, &engine).await;
    assert_eq!(source.status, "failed");
    let source_script_id = source.final_script_resource_id.unwrap();

    let preview = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source_script_id,
            RetryStudyRequest {
                client_retry_id: "retry-1".into(),
            },
        )
        .await
        .unwrap();
    assert!(preview.retry_preview_id.is_none());
    let retry = preview.workflow.unwrap();
    assert_eq!(retry.source_workflow_id, Some(source.id));
    assert_eq!(retry.corrections_used, 0);
    assert_eq!(retry.correction_budget, source.correction_budget);

    author.enqueue(AuthorTurn::Script("result = test(table, para);".into()));
    engine.enqueue(EngineScenario::IdSuccess(vec![IdDatasetFixture {
        name: "result".into(),
        ids: vec!["row-1".into()],
    }]));
    app.state
        .managed_workflow_service
        .execute_confirmed(&user, retry.id)
        .await
        .unwrap();
    let retry = app
        .state
        .managed_workflow_service
        .get(&user, retry.id)
        .await
        .unwrap();
    assert_eq!(retry.status, "succeeded", "{retry:?}");
    let retry_script_id = retry.final_script_resource_id.unwrap();
    assert_ne!(retry_script_id, source_script_id);

    let mut conn = app.pool.get().unwrap();
    #[derive(QueryableByName)]
    struct Derived {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
        source_script_resource_id: Option<i32>,
    }
    let derived =
        diesel::sql_query("SELECT source_script_resource_id FROM script_resources WHERE id=?1")
            .bind::<diesel::sql_types::Integer, _>(retry_script_id)
            .get_result::<Derived>(&mut conn)
            .unwrap();
    assert_eq!(derived.source_script_resource_id, Some(source_script_id));
    drop(conn);
    assert_eq!(
        app.state
            .managed_workflow_service
            .get(&user, source.id)
            .await
            .unwrap()
            .final_script_resource_id,
        Some(source_script_id)
    );
}

#[tokio::test]
async fn changed_inputs_require_confirmation_and_freeze_the_new_active_model() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    let app = common::TestApp::spawn_minimal_with_engine_and_agent_factory(
        engine.clone(),
        Arc::new(ScriptedAgentFactory::default()),
        author.clone(),
    )
    .await;
    let user = common::test_user("alice@example.com");
    let source = failed_source(&app, &user, &author, &engine).await;
    let source_script_id = source.final_script_resource_id.unwrap();
    let session_id = app
        .state
        .run_service
        .find(&user, source.run_id)
        .await
        .unwrap()
        .session_id;

    let draft = common::publish_model_bytes(
        &app,
        &user,
        session_id,
        "retry-new-input",
        "table.csv",
        b"id\nrow-2\n",
        None,
        Some("table"),
    )
    .await;
    let version = app
        .state
        .version_service
        .publish(
            &user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "retry-new-version".into(),
                description: Some("new retry input".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            &user,
            version.version.version.id,
            ActivateVersionRequest {
                discard_draft: false,
                expected_draft: None,
            },
        )
        .await
        .unwrap();

    let preview = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source_script_id,
            RetryStudyRequest {
                client_retry_id: "retry-changed".into(),
            },
        )
        .await
        .unwrap();
    assert!(preview.workflow.is_none());
    assert!(!preview.changes.as_array().unwrap().is_empty());
    let token = preview.retry_preview_id.unwrap();
    let retry = app
        .state
        .managed_workflow_service
        .confirm_retry(&user, &token, ConfirmRetryRequest { confirmed: true })
        .await
        .unwrap();
    assert_eq!(retry.source_workflow_id, Some(source.id));
    let run = app
        .state
        .run_service
        .find(&user, retry.run_id)
        .await
        .unwrap();
    assert_eq!(run.model_version_id, Some(version.version.version.id));
    assert_ne!(
        run.model_content_hash,
        app.state
            .run_service
            .find(&user, source.run_id)
            .await
            .unwrap()
            .model_content_hash
    );
}
