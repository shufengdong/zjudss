mod common;

use arrow_array::{Array, Float64Array, StringArray, UInt32Array};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::agent::{ScriptAuthor, ScriptGenerationRequest, ScriptRepairRequest};
use zjudss::application::{
    CandidateDecision, CandidatePurpose, ConfirmStudyRequest, CreateRunRequest,
    LockedStudyContract, PrepareStudyRequest, ReplacementAction, SelectStudyResultsRequest,
    StorageLimits, UpdateStudyStructRequest, WorkflowInputSnapshot,
};
use zjudss::db::repositories::{
    ArtifactDraft, ConfirmStartOutcome, ManagedWorkflowRepo, ManagedWorkflowRepoError, RepoError,
    WorkflowOverlayInput,
};
use zjudss::storage::StagedArtifact;
use zjudss::testing::{AuthorTurn, ScriptedAuthor};

use common::TestApp;

type StagedOverlayParts = (
    Vec<WorkflowOverlayInput>,
    Vec<ArtifactDraft>,
    Vec<StagedArtifact>,
);

async fn stage_overlays(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    prepared: &zjudss::application::PreparedStudyConfirmation,
) -> StagedOverlayParts {
    app.state
        .stage_struct_overlays_for_test(user, &prepared.struct_overlays)
        .await
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn confirm_staged(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    prepared: zjudss::application::PreparedStudyConfirmation,
    inputs: Vec<WorkflowOverlayInput>,
    drafts: Vec<ArtifactDraft>,
    mut staged: Vec<StagedArtifact>,
    operation: &str,
    now_ms: i64,
) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError> {
    let store = app.state.artifact_service.store().clone();
    let command = prepared
        .into_start_command(operation.into(), 3, "test".into(), "[]".into(), now_ms)
        .unwrap();
    ManagedWorkflowRepo::new(app.pool.clone())
        .confirm_and_start_with_overlay_artifacts(
            &user.id,
            command,
            inputs,
            drafts,
            move |index, path| {
                store
                    .publish_for_test(&mut staged[index], path)
                    .map_err(|error| RepoError::Storage(error.to_string()))
            },
        )
        .await
}

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
        .upload_confirmation_service
        .confirm(
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
    assert!(workspace.draft.is_none());
    assert!(workspace.active.is_some());
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
                byte_limit: usage.used_bytes,
                warning_ratio: usage.warning_ratio,
            },
        )
        .await
        .unwrap();
    let replay = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(&user, configuration.id, request)
        .await
        .unwrap();
    app.state
        .artifact_service
        .set_limits(
            user.storage_scope_id,
            StorageLimits {
                artifact_limit: i64::MAX,
                byte_limit: i64::MAX,
                warning_ratio: usage.warning_ratio,
            },
        )
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
async fn ordinary_struct_overrides_freeze_without_publishing_a_resource() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "private struct overlay", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    let source_resource_id = para.binding.resource_id.clone().unwrap();
    let before = app
        .state
        .data_resource_service
        .list(&user, session.id, None, 100)
        .await
        .unwrap()
        .resources;
    let mut values = para.struct_card.as_ref().unwrap().values.clone();
    values["max_iter"] = json!(37);
    values["tol"] = json!(1e-7);
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
            },
        )
        .await
        .unwrap();
    let selected = app
        .state
        .study_configuration_service
        .select_results(
            &user,
            edited.id,
            SelectStudyResultsRequest {
                expected_revision: edited.revision,
                results: vec!["bus_voltage".into(), "load_state".into()],
            },
        )
        .await
        .unwrap();
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            selected.id,
            ConfirmStudyRequest {
                expected_revision: selected.revision,
                client_operation_id: "private-overlay-confirm".into(),
            },
        )
        .await
        .unwrap();
    let after = app
        .state
        .data_resource_service
        .list(&user, session.id, None, 100)
        .await
        .unwrap()
        .resources;
    assert_eq!(
        after
            .iter()
            .map(|resource| (&resource.id, &resource.alias, &resource.status))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|resource| (&resource.id, &resource.alias, &resource.status))
            .collect::<Vec<_>>()
    );

    #[derive(QueryableByName)]
    struct FrozenPara {
        #[diesel(sql_type = diesel::sql_types::Text)]
        resource_id: String,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        artifact_id: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        snapshot_json: String,
    }
    let frozen = diesel::sql_query(
        "SELECT resource_id,artifact_id,snapshot_json FROM workflow_input_snapshots WHERE workflow_id=?1 AND input_name='para'",
    )
    .bind::<diesel::sql_types::Integer, _>(workflow.id)
    .get_result::<FrozenPara>(&mut app.pool.get().unwrap())
    .unwrap();
    assert_eq!(frozen.resource_id, source_resource_id);
    let snapshot: Value = serde_json::from_str(&frozen.snapshot_json).unwrap();
    assert_eq!(snapshot["source"]["resourceId"], source_resource_id);
    assert_eq!(snapshot["effective"]["userOverrides"]["max_iter"], 37);
    assert_eq!(snapshot["effective"]["userOverrides"]["tol"], 1e-7);
    assert_eq!(
        snapshot["effective"]["selectorOverrides"]["result"],
        "bus_voltage,load_state"
    );
    assert_eq!(
        snapshot["effective"]["overrides"]["result"],
        "bus_voltage,load_state"
    );
    assert_eq!(snapshot["effective"]["values"]["max_iter"], 37);

    let file = app
        .state
        .artifact_service
        .open_owned(user.storage_scope_id, frozen.artifact_id)
        .await
        .unwrap();
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    let batch = reader.next().unwrap().unwrap();
    let result_column = batch.schema().index_of("result").unwrap();
    let result = batch
        .column(result_column)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(!result.is_null(0));
    assert_eq!(result.value(0), "bus_voltage,load_state");
    let max_iter = batch
        .column(batch.schema().index_of("max_iter").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(max_iter.value(0), 37);
    let tol = batch
        .column(batch.schema().index_of("tol").unwrap())
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert_eq!(tol.value(0), 1e-7);

    let next = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"再次求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let next_para = next
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    assert_eq!(next_para.resource_options.len(), 1);
    assert_eq!(
        next_para.binding.resource_id.as_deref(),
        Some(source_resource_id.as_str())
    );
}

#[tokio::test]
async fn byte_identical_private_values_reuse_the_source_artifact() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "identical struct overlay", 1)
        .await
        .unwrap();
    let initial = ready_study(&app, &user, session.id).await;
    let original_id = initial
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap()
        .binding
        .resource_id
        .clone()
        .unwrap();
    let original = app
        .state
        .data_resource_service
        .get(&user, &original_id)
        .await
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(&user, &original_id, Some(2), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    values.insert("result".into(), json!("bus_voltage"));
    let preview = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &original_id,
            &original.content_hash,
            "frames.para",
            Value::Object(values),
        )
        .await
        .unwrap();
    let current = app
        .state
        .data_resource_service
        .confirm_structured_replacement(&user, &preview.preview_token, ReplacementAction::Replace)
        .await
        .unwrap()
        .unwrap();
    let source_artifact_id = zjudss::db::schema::data_resources::table
        .find(&current.id)
        .select(zjudss::db::schema::data_resources::normalized_artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            configuration.id,
            ConfirmStudyRequest {
                expected_revision: configuration.revision,
                client_operation_id: "identical-overlay-confirm".into(),
            },
        )
        .await
        .unwrap();
    let frozen_artifact_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(workflow.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(frozen_artifact_id, source_artifact_id);
}

#[tokio::test]
async fn source_less_struct_confirmation_owns_a_private_artifact() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "source-less private struct", 1)
        .await
        .unwrap();
    let initial = ready_study(&app, &user, session.id).await;
    let para = initial
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    let source_id = para.binding.resource_id.clone().unwrap();
    let source_page = app
        .state
        .data_resource_service
        .rows(&user, &source_id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let source_values = Value::Object(
        source_page
            .columns
            .into_iter()
            .zip(source_page.rows.into_iter().next().unwrap())
            .collect(),
    );
    diesel::update(zjudss::db::schema::data_resources::table.find(&source_id))
        .set(zjudss::db::schema::data_resources::status.eq("replaced"))
        .execute(&mut app.pool.get().unwrap())
        .unwrap();

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session.id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"使用私有参数求解潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let completed = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values: source_values,
            },
        )
        .await
        .unwrap();
    let workflow = app
        .state
        .managed_workflow_service
        .confirm_configuration_and_start(
            &user,
            completed.id,
            ConfirmStudyRequest {
                expected_revision: completed.revision,
                client_operation_id: "source-less-private-confirm".into(),
            },
        )
        .await
        .unwrap();

    #[derive(QueryableByName)]
    struct Frozen {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        resource_id: Option<String>,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        artifact_id: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        content_hash: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        snapshot_json: String,
    }
    let frozen = diesel::sql_query(
        "SELECT resource_id,artifact_id,content_hash,snapshot_json FROM workflow_input_snapshots WHERE workflow_id=?1 AND input_name='para'",
    )
    .bind::<diesel::sql_types::Integer, _>(workflow.id)
    .get_result::<Frozen>(&mut app.pool.get().unwrap())
    .unwrap();
    assert!(frozen.resource_id.is_none());
    let snapshot: Value = serde_json::from_str(&frozen.snapshot_json).unwrap();
    assert!(snapshot["source"].is_null());
    assert!(snapshot["effective"]["values"].is_object());
    let run_snapshot_json = zjudss::db::schema::simulation_runs::table
        .filter(zjudss::db::schema::simulation_runs::managed_workflow_id.eq(workflow.id))
        .select(zjudss::db::schema::simulation_runs::input_snapshot_json)
        .first::<String>(&mut app.pool.get().unwrap())
        .unwrap();
    let run_snapshot: Value = serde_json::from_str(&run_snapshot_json).unwrap();
    let run_para = run_snapshot["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["inputId"] == "para")
        .unwrap();
    assert!(run_para["resourceId"].is_null());
    assert_eq!(run_para["contentHash"], frozen.content_hash);

    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            workflow.id,
            "generation",
            None,
            "bus_voltage = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);",
            Some("test".into()),
        )
        .await
        .unwrap();
    let terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, workflow.id, candidate.attempts[0].id, true, None)
        .await
        .unwrap();
    let rerun = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "source-less-private-rerun".into(),
            },
        )
        .await
        .unwrap();
    let rerun_inputs: Value = serde_json::from_str(&rerun.run.input_snapshot_json).unwrap();
    let rerun_para = rerun_inputs["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["alias"] == "para")
        .unwrap();
    assert!(rerun_para["resourceId"].is_null());
    assert_eq!(
        rerun_para["artifactId"].as_i64(),
        Some(i64::from(frozen.artifact_id))
    );

    app.state
        .session_repo
        .delete(&user.id, session.id)
        .await
        .unwrap();
    let remaining = zjudss::db::schema::storage_scope_artifacts::table
        .filter(
            zjudss::db::schema::storage_scope_artifacts::storage_scope_id.eq(user.storage_scope_id),
        )
        .filter(zjudss::db::schema::storage_scope_artifacts::artifact_id.eq(frozen.artifact_id))
        .count()
        .get_result::<i64>(&mut app.pool.get().unwrap())
        .unwrap();
    assert_eq!(remaining, 0);
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
    let (inputs, drafts, staged) = stage_overlays(&app, &user, &prepared).await;
    assert_eq!(prepared.struct_overlays.len(), 1);
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE workspaces SET active_version_id=NULL WHERE session_id=?1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .execute(&mut conn)
        .unwrap();
    drop(conn);

    let error = confirm_staged(
        &app,
        &user,
        prepared,
        inputs,
        drafts,
        staged,
        "active-race",
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
async fn confirmation_missing_owned_artifact_rolls_back_without_partial_workflow() {
    #[derive(QueryableByName)]
    struct ArtifactId {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        artifact_id: i32,
    }

    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "missing owned artifact", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap();
    let (inputs, drafts, staged) = stage_overlays(&app, &user, &prepared).await;

    let mut conn = app.pool.get().unwrap();
    let artifact_id = diesel::sql_query("SELECT d.artifact_id FROM model_dataframes d JOIN model_versions v ON v.id=d.version_id JOIN workspaces w ON w.id=v.workspace_id WHERE w.session_id=?1 ORDER BY d.id LIMIT 1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .get_result::<ArtifactId>(&mut conn)
        .unwrap()
        .artifact_id;
    diesel::delete(
        zjudss::db::schema::storage_scope_artifacts::table
            .find((user.storage_scope_id, artifact_id)),
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);

    let error = confirm_staged(
        &app,
        &user,
        prepared,
        inputs,
        drafts,
        staged,
        "missing-owned-artifact",
        2,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        ManagedWorkflowRepoError::ConfigurationStale
    ));

    let mut conn = app.pool.get().unwrap();
    let status = zjudss::db::schema::study_configurations::table
        .find(configuration.id)
        .select(zjudss::db::schema::study_configurations::status)
        .first::<String>(&mut conn)
        .unwrap();
    let workflow_count = zjudss::db::schema::managed_workflows::table
        .filter(zjudss::db::schema::managed_workflows::configuration_id.eq(Some(configuration.id)))
        .count()
        .get_result::<i64>(&mut conn)
        .unwrap();
    let run_count = zjudss::db::schema::simulation_runs::table
        .filter(zjudss::db::schema::simulation_runs::session_id.eq(session.id))
        .filter(zjudss::db::schema::simulation_runs::managed_workflow_id.is_not_null())
        .count()
        .get_result::<i64>(&mut conn)
        .unwrap();
    let input_count = zjudss::db::schema::workflow_input_snapshots::table
        .count()
        .get_result::<i64>(&mut conn)
        .unwrap();
    assert_eq!(status, "ready_to_confirm");
    assert_eq!((workflow_count, run_count, input_count), (0, 0, 0));
    drop(conn);
}

#[tokio::test]
async fn later_overlay_failure_cleans_earlier_material() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "later overlay cleanup", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let mut prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap();
    let mut invalid_later_overlay = prepared.struct_overlays[0].clone();
    invalid_later_overlay.input_id = "invalid-later-overlay".into();
    invalid_later_overlay.schema_ref = "frames.missing".into();
    prepared.struct_overlays.push(invalid_later_overlay);

    let before_usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let before_counts = {
        let mut conn = app.pool.get().unwrap();
        (
            zjudss::db::schema::storage_scope_artifacts::table
                .filter(
                    zjudss::db::schema::storage_scope_artifacts::storage_scope_id
                        .eq(user.storage_scope_id),
                )
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            zjudss::db::schema::managed_workflows::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            zjudss::db::schema::simulation_runs::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            zjudss::db::schema::workflow_input_snapshots::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
        )
    };

    let error = app
        .state
        .stage_struct_overlays_for_test(&user, &prepared.struct_overlays)
        .await
        .unwrap_err();
    assert_eq!(error.0, "configuration_stale");

    let after_usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    let after_counts = (
        zjudss::db::schema::storage_scope_artifacts::table
            .filter(
                zjudss::db::schema::storage_scope_artifacts::storage_scope_id
                    .eq(user.storage_scope_id),
            )
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        zjudss::db::schema::managed_workflows::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        zjudss::db::schema::simulation_runs::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        zjudss::db::schema::workflow_input_snapshots::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
    );
    assert_eq!(after_counts, before_counts);
    assert_eq!(after_usage.used_artifacts, before_usage.used_artifacts);
    assert_eq!(after_usage.used_bytes, before_usage.used_bytes);
}

#[tokio::test]
async fn private_struct_overlay_rejects_a_stale_source_hash() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "stale private overlay", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap();
    let (mut inputs, drafts, staged) = stage_overlays(&app, &user, &prepared).await;
    inputs[0].source_content_hash = Some("0".repeat(64));

    let error = confirm_staged(
        &app,
        &user,
        prepared,
        inputs,
        drafts,
        staged,
        "stale-private-overlay",
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
async fn ordinary_struct_override_keeps_the_source_resource_live() {
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
    assert_eq!(frozen.resource_id, old_id);
    assert_eq!(frozen.old_status, "live");
}

#[tokio::test]
async fn rerun_rejects_same_alias_resource_for_struct_snapshot_without_effective_overlay() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "unchanged para rerun", 1)
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
                client_operation_id: "unchanged-para-confirm".into(),
            },
        )
        .await
        .unwrap();
    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            workflow.id,
            "generation",
            None,
            "bus_voltage = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);",
            Some("test".into()),
        )
        .await
        .unwrap();
    let terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, workflow.id, candidate.attempts[0].id, true, None)
        .await
        .unwrap();

    let mut conn = app.pool.get().unwrap();
    let frozen = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(workflow.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .first::<zjudss::db::models::WorkflowInputSnapshotRecord>(&mut conn)
        .unwrap();
    let snapshot: Value = serde_json::from_str(&frozen.snapshot_json).unwrap();
    let source = snapshot["source"].clone();
    let source_resource_id = source["resourceId"].as_str().unwrap().to_string();
    let source_artifact_id = source["artifactId"].as_i64().unwrap() as i32;
    let source_content_hash = source["contentHash"].as_str().unwrap().to_string();
    diesel::sql_query("DROP TRIGGER immutable_workflow_inputs")
        .execute(&mut conn)
        .unwrap();
    diesel::update(zjudss::db::schema::workflow_input_snapshots::table.find(frozen.id))
        .set((
            zjudss::db::schema::workflow_input_snapshots::artifact_id.eq(source_artifact_id),
            zjudss::db::schema::workflow_input_snapshots::content_hash.eq(source_content_hash),
            zjudss::db::schema::workflow_input_snapshots::snapshot_json.eq(source.to_string()),
        ))
        .execute(&mut conn)
        .unwrap();
    diesel::sql_query("CREATE TRIGGER immutable_workflow_inputs BEFORE UPDATE ON workflow_input_snapshots BEGIN SELECT RAISE(ABORT,'workflow inputs are immutable'); END")
        .execute(&mut conn)
        .unwrap();

    let mut unrelated = zjudss::db::schema::data_resources::table
        .find(&source_resource_id)
        .first::<zjudss::db::models::DataResourceRecord>(&mut conn)
        .unwrap();
    diesel::update(zjudss::db::schema::data_resources::table.find(&source_resource_id))
        .set((
            zjudss::db::schema::data_resources::alias.eq("para_detached"),
            zjudss::db::schema::data_resources::normalized_alias.eq("para_detached"),
            zjudss::db::schema::data_resources::status.eq("replaced"),
            zjudss::db::schema::data_resources::replaced_by_resource_id.eq(Option::<String>::None),
        ))
        .execute(&mut conn)
        .unwrap();
    unrelated.id = "dr_unrelated_unchanged_para".into();
    unrelated.status = "live".into();
    unrelated.replaced_by_resource_id = None;
    unrelated.lineage_json = json!({}).to_string();
    diesel::insert_into(zjudss::db::schema::data_resources::table)
        .values(unrelated)
        .execute(&mut conn)
        .unwrap();
    drop(conn);

    let error = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "unrelated-unchanged-para-rerun".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "冻结输入的来源已经变化");
    assert!(matches!(
        error,
        zjudss::application::RunError::ConfigurationStale
    ));
}

#[tokio::test]
async fn private_struct_override_keeps_the_facade_alias_for_engine_injection() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "save para as new", 1)
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
                client_operation_id: "save-as-new-confirm".into(),
            },
        )
        .await
        .unwrap();

    #[derive(QueryableByName)]
    struct FrozenAliases {
        #[diesel(sql_type = diesel::sql_types::Text)]
        resource_id: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        input_alias: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        resource_alias: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        selected_results: String,
    }
    let mut conn = app.pool.get().unwrap();
    let aliases = diesel::sql_query(
        "SELECT i.resource_id,i.input_alias,json_extract(i.snapshot_json,'$.resourceAlias') resource_alias,json_extract(w.locked_contract_json,'$.structs[0].fields.result') selected_results FROM workflow_input_snapshots i JOIN managed_workflows w ON w.id=i.workflow_id WHERE i.workflow_id=?1 AND i.input_name='para'",
    )
    .bind::<diesel::sql_types::Integer, _>(workflow.id)
    .get_result::<FrozenAliases>(&mut conn)
    .unwrap();
    assert_eq!(aliases.input_alias, "para");
    assert_eq!(aliases.resource_alias, aliases.input_alias);
    assert_eq!(aliases.selected_results, "bus_voltage");
    drop(conn);

    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            workflow.id,
            "generation",
            None,
            "bus_voltage = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);",
            Some("test".into()),
        )
        .await
        .unwrap();
    let terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, workflow.id, candidate.attempts[0].id, true, None)
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    let mut unrelated = zjudss::db::schema::data_resources::table
        .find(&aliases.resource_id)
        .first::<zjudss::db::models::DataResourceRecord>(&mut conn)
        .unwrap();
    diesel::update(zjudss::db::schema::data_resources::table.find(&aliases.resource_id))
        .set((
            zjudss::db::schema::data_resources::alias.eq("para_detached"),
            zjudss::db::schema::data_resources::normalized_alias.eq("para_detached"),
            zjudss::db::schema::data_resources::status.eq("replaced"),
        ))
        .execute(&mut conn)
        .unwrap();
    unrelated.id = "dr_unrelated_rerun_para".into();
    unrelated.status = "live".into();
    unrelated.replaced_by_resource_id = None;
    unrelated.lineage_json = json!({}).to_string();
    diesel::insert_into(zjudss::db::schema::data_resources::table)
        .values(unrelated)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let unrelated_error = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "unrelated-same-alias-rerun".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unrelated_error.to_string(), "冻结输入的来源已经变化");
    assert!(matches!(
        unrelated_error,
        zjudss::application::RunError::ConfigurationStale
    ));
    let mut conn = app.pool.get().unwrap();
    diesel::delete(zjudss::db::schema::data_resources::table.find("dr_unrelated_rerun_para"))
        .execute(&mut conn)
        .unwrap();
    diesel::update(zjudss::db::schema::data_resources::table.find(&aliases.resource_id))
        .set((
            zjudss::db::schema::data_resources::alias.eq("para"),
            zjudss::db::schema::data_resources::normalized_alias.eq("para"),
            zjudss::db::schema::data_resources::status.eq("live"),
        ))
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let current_para = app
        .state
        .data_resource_service
        .get(&user, &aliases.resource_id)
        .await
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(&user, &aliases.resource_id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut latest_values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    latest_values.insert("max_iter".into(), json!(77));
    let preview = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &aliases.resource_id,
            &current_para.content_hash,
            "frames.para",
            Value::Object(latest_values),
        )
        .await
        .unwrap();
    let latest_para = app
        .state
        .data_resource_service
        .confirm_structured_replacement(&user, &preview.preview_token, ReplacementAction::Replace)
        .await
        .unwrap()
        .unwrap();
    let rerun = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "save-as-new-rerun".into(),
            },
        )
        .await
        .unwrap();
    let frozen: Value = serde_json::from_str(&rerun.run.input_snapshot_json).unwrap();
    assert_eq!(frozen["resources"][0]["alias"], "para");
    assert_eq!(frozen["resources"][0]["resourceId"], latest_para.id);
    let rerun_para = frozen["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|resource| resource["alias"] == "para")
        .unwrap();
    let file = app
        .state
        .artifact_service
        .open_owned(
            user.storage_scope_id,
            rerun_para["artifactId"].as_i64().unwrap() as i32,
        )
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

    let page = app
        .state
        .data_resource_service
        .rows(&user, &latest_para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut drifted_values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    drifted_values.insert("max_iter".into(), json!(88));
    let preview = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &latest_para.id,
            &latest_para.content_hash,
            "frames.para",
            Value::Object(drifted_values),
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .confirm_structured_replacement(&user, &preview.preview_token, ReplacementAction::Replace)
        .await
        .unwrap();
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
                byte_limit: usage.used_bytes,
                warning_ratio: usage.warning_ratio,
            },
        )
        .await
        .unwrap();
    let replay = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "save-as-new-rerun".into(),
            },
        )
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.run.id, rerun.run.id);
}

#[tokio::test]
async fn legacy_private_overlay_snapshot_reruns_with_merged_overrides() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "legacy private rerun", 1)
        .await
        .unwrap();
    let configuration = ready_study(&app, &user, session.id).await;
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    let source_id = para.binding.resource_id.clone().unwrap();
    let mut values = para.struct_card.as_ref().unwrap().values.clone();
    let expected_line_model = if values["line_model"]
        .as_str()
        .unwrap_or("exact")
        .eq_ignore_ascii_case("exact")
    {
        "modified"
    } else {
        "exact"
    };
    values["line_model"] = json!(expected_line_model);
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
                client_operation_id: "legacy-private-rerun-source".into(),
            },
        )
        .await
        .unwrap();
    let candidate = app
        .state
        .managed_workflow_service
        .record_candidate_script(
            &user,
            workflow.id,
            "generation",
            None,
            "bus_voltage = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);",
            Some("test".into()),
        )
        .await
        .unwrap();
    let terminal = app
        .state
        .managed_workflow_service
        .finalize_with_script(&user, workflow.id, candidate.attempts[0].id, true, None)
        .await
        .unwrap();

    let mut conn = app.pool.get().unwrap();
    let raw = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(workflow.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::snapshot_json)
        .first::<String>(&mut conn)
        .unwrap();
    let mut snapshot: Value = serde_json::from_str(&raw).unwrap();
    snapshot["effective"]
        .as_object_mut()
        .unwrap()
        .remove("userOverrides");
    snapshot["effective"]
        .as_object_mut()
        .unwrap()
        .remove("selectorOverrides");
    diesel::sql_query("DROP TRIGGER immutable_workflow_inputs")
        .execute(&mut conn)
        .unwrap();
    diesel::update(
        zjudss::db::schema::workflow_input_snapshots::table
            .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(workflow.id))
            .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para")),
    )
    .set(zjudss::db::schema::workflow_input_snapshots::snapshot_json.eq(snapshot.to_string()))
    .execute(&mut conn)
    .unwrap();
    drop(conn);

    let source = app
        .state
        .data_resource_service
        .get(&user, &source_id)
        .await
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(&user, &source_id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut source_values = page
        .columns
        .into_iter()
        .zip(page.rows.into_iter().next().unwrap())
        .collect::<serde_json::Map<_, _>>();
    source_values.insert("max_iter".into(), json!(77));
    let replacement = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &source_id,
            &source.content_hash,
            "frames.para",
            Value::Object(source_values),
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .confirm_structured_replacement(
            &user,
            &replacement.preview_token,
            ReplacementAction::Replace,
        )
        .await
        .unwrap();
    let rerun = app
        .state
        .run_service
        .create(
            &user,
            terminal.final_script_resource_id.unwrap(),
            CreateRunRequest {
                client_run_id: "legacy-private-rerun".into(),
            },
        )
        .await
        .unwrap();
    let frozen: Value = serde_json::from_str(&rerun.run.input_snapshot_json).unwrap();
    let para = frozen["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|resource| resource["alias"] == "para")
        .unwrap();
    let file = app
        .state
        .artifact_service
        .open_owned(
            user.storage_scope_id,
            para["artifactId"].as_i64().unwrap() as i32,
        )
        .await
        .unwrap();
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .build()
        .unwrap();
    let batch = reader.next().unwrap().unwrap();
    let line_model = batch
        .column(batch.schema().index_of("line_model").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(line_model.value(0), expected_line_model);
    let result = batch
        .column(batch.schema().index_of("result").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(result.value(0), "bus_voltage");
}

#[tokio::test]
async fn concurrent_confirmation_replay_releases_unconsumed_private_artifact_reference() {
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
    let (first_inputs, first_drafts, first_staged) = stage_overlays(&app, &user, &prepared).await;
    let (replay_inputs, replay_drafts, replay_staged) =
        stage_overlays(&app, &user, &prepared).await;
    let first = confirm_staged(
        &app,
        &user,
        prepared.clone(),
        first_inputs,
        first_drafts,
        first_staged,
        "replacement-replay",
        2,
    )
    .await
    .unwrap();
    let replay = confirm_staged(
        &app,
        &user,
        prepared,
        replay_inputs,
        replay_drafts,
        replay_staged,
        "replacement-replay",
        3,
    )
    .await
    .unwrap();
    assert!(first.created);
    assert!(!replay.created);
    assert_eq!(first.aggregate.workflow.id, replay.aggregate.workflow.id);
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
    assert_eq!(refs.resource_refs, 0);
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
