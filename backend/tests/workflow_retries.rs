mod common;

use std::sync::Arc;

use arrow_array::{Array, Float64Array, StringArray, UInt32Array};
use diesel::prelude::*;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::{Value, json};
use zjudss::application::{
    ActivateVersionRequest, CandidateDecision, CandidatePurpose, ConfirmRetryRequest,
    ConfirmStudyRequest, PrepareStudyRequest, PublishDraftRequest, RetryStudyRequest,
    StorageLimits, UpdateStudyStructRequest,
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
    for index in 0..4 {
        author.enqueue(AuthorTurn::Script(format!(
            "result = test(table, para); // source attempt {index}"
        )));
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
    source_less: bool,
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
        .upload_confirmation_service
        .confirm(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            None,
            "overlay-retry-import",
        )
        .await
        .unwrap();
    let para = app
        .state
        .data_resource_service
        .list(user, session.id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(user, &para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows.into_iter().next().unwrap())
        .collect::<serde_json::Map<_, _>>();
    values.insert("max_iter".into(), json!(37));
    values.insert("tol".into(), json!(1e-7));
    if source_less {
        diesel::update(zjudss::db::schema::data_resources::table.find(&para.id))
            .set(zjudss::db::schema::data_resources::status.eq("replaced"))
            .execute(&mut app.pool.get().unwrap())
            .unwrap();
    }
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
    let configuration = app
        .state
        .study_configuration_service
        .update_struct(
            user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values: Value::Object(values),
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
                client_operation_id: if source_less {
                    "source-less-overlay-retry-source".into()
                } else {
                    "overlay-retry-source".into()
                },
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

fn publish_unrelated_para_with_lineage(
    app: &common::TestApp,
    source_resource_id: &str,
    new_resource_id: &str,
    lineage: Value,
) {
    use zjudss::db::models::DataResourceRecord;
    use zjudss::db::schema::data_resources;

    let mut conn = app.pool.get().unwrap();
    let mut replacement = data_resources::table
        .find(source_resource_id)
        .first::<DataResourceRecord>(&mut conn)
        .unwrap();
    diesel::update(data_resources::table.find(source_resource_id))
        .set((
            data_resources::alias.eq(format!("{}_detached", replacement.alias)),
            data_resources::normalized_alias
                .eq(format!("{}_detached", replacement.normalized_alias)),
            data_resources::status.eq("replaced"),
            data_resources::replaced_by_resource_id.eq(Option::<String>::None),
        ))
        .execute(&mut conn)
        .unwrap();
    replacement.id = new_resource_id.into();
    replacement.status = "live".into();
    replacement.replaced_by_resource_id = None;
    replacement.lineage_json = lineage.to_string();
    replacement.created_at_ms += 1;
    replacement.updated_at_ms += 1;
    diesel::insert_into(data_resources::table)
        .values(replacement)
        .execute(&mut conn)
        .unwrap();
}

fn frozen_para(
    app: &common::TestApp,
    workflow_id: i32,
) -> zjudss::db::models::WorkflowInputSnapshotRecord {
    zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(workflow_id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .first(&mut app.pool.get().unwrap())
        .unwrap()
}

async fn replace_live_para(
    app: &common::TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
    max_iter: u32,
) {
    let para = app
        .state
        .data_resource_service
        .list(user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(user, &para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows.into_iter().next().unwrap())
        .collect::<serde_json::Map<_, _>>();
    values.insert("max_iter".into(), json!(max_iter));
    let preview = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            user,
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
            user,
            &preview.preview_token,
            zjudss::application::ReplacementAction::Replace,
        )
        .await
        .unwrap();
}

#[derive(Clone, Copy, Debug)]
enum SuccessorCase {
    Unchanged,
    ExplicitReplacement,
    SourceLess,
    LegacyMergedOverrides,
    SameAliasImpostor,
    Cycle,
    OverDepth,
}

impl SuccessorCase {
    fn name(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::ExplicitReplacement => "explicit-replacement",
            Self::SourceLess => "source-less",
            Self::LegacyMergedOverrides => "legacy-merged-overrides",
            Self::SameAliasImpostor => "same-alias-impostor",
            Self::Cycle => "cycle",
            Self::OverDepth => "over-depth",
        }
    }

    fn rejects(self) -> bool {
        matches!(
            self,
            Self::SameAliasImpostor | Self::Cycle | Self::OverDepth
        )
    }
}

#[tokio::test]
async fn retry_and_rerun_resolve_the_same_struct_successor_matrix() {
    use zjudss::application::{CreateRunRequest, RunError};

    for case in [
        SuccessorCase::Unchanged,
        SuccessorCase::ExplicitReplacement,
        SuccessorCase::SourceLess,
        SuccessorCase::LegacyMergedOverrides,
        SuccessorCase::SameAliasImpostor,
        SuccessorCase::Cycle,
        SuccessorCase::OverDepth,
    ] {
        let app = common::TestApp::spawn().await;
        let user = common::test_user("alice@example.com");
        let source =
            failed_overlay_source(&app, &user, matches!(case, SuccessorCase::SourceLess)).await;
        let source_script_id = source.final_script_resource_id.unwrap();
        let source_input = frozen_para(&app, source.id);
        let original_source_snapshot: Value =
            serde_json::from_str(&source_input.snapshot_json).unwrap();
        let expected_merged_overrides = original_source_snapshot["effective"]["overrides"].clone();
        let expected_user_overrides = original_source_snapshot["effective"]["userOverrides"]
            .as_object()
            .map(|_| original_source_snapshot["effective"]["userOverrides"].clone())
            .unwrap_or_else(|| json!({}));
        let expected_selector_overrides =
            original_source_snapshot["effective"]["selectorOverrides"]
                .as_object()
                .map(|_| original_source_snapshot["effective"]["selectorOverrides"].clone())
                .unwrap_or_else(|| json!({}));
        let session_id = app
            .state
            .run_service
            .find(&user, source.run_id)
            .await
            .unwrap()
            .session_id;

        if matches!(case, SuccessorCase::LegacyMergedOverrides) {
            let mut snapshot: Value = serde_json::from_str(&source_input.snapshot_json).unwrap();
            let effective = snapshot["effective"].as_object_mut().unwrap();
            effective.remove("userOverrides");
            effective.remove("selectorOverrides");
            let mut conn = app.pool.get().unwrap();
            diesel::sql_query("DROP TRIGGER immutable_workflow_inputs")
                .execute(&mut conn)
                .unwrap();
            diesel::update(
                zjudss::db::schema::workflow_input_snapshots::table.find(source_input.id),
            )
            .set(
                zjudss::db::schema::workflow_input_snapshots::snapshot_json
                    .eq(snapshot.to_string()),
            )
            .execute(&mut conn)
            .unwrap();
        }

        match case {
            SuccessorCase::ExplicitReplacement => {
                replace_live_para(&app, &user, session_id, 77).await;
            }
            SuccessorCase::LegacyMergedOverrides => {
                replace_live_para(&app, &user, session_id, 99).await;
            }
            SuccessorCase::SameAliasImpostor => {
                publish_unrelated_para_with_lineage(
                    &app,
                    source_input.resource_id.as_deref().unwrap(),
                    "dr_matrix_impostor",
                    json!({}),
                );
            }
            SuccessorCase::Cycle => {
                let source_resource_id = source_input.resource_id.clone().unwrap();
                publish_unrelated_para_with_lineage(
                    &app,
                    &source_resource_id,
                    "dr_matrix_cycle",
                    json!({"replacesResourceId":source_resource_id}),
                );
                let mut conn = app.pool.get().unwrap();
                diesel::update(zjudss::db::schema::data_resources::table.find(&source_resource_id))
                    .set(
                        zjudss::db::schema::data_resources::replaced_by_resource_id
                            .eq(Some("dr_matrix_cycle".to_string())),
                    )
                    .execute(&mut conn)
                    .unwrap();
                diesel::update(zjudss::db::schema::data_resources::table.find("dr_matrix_cycle"))
                    .set((
                        zjudss::db::schema::data_resources::status.eq("replaced"),
                        zjudss::db::schema::data_resources::replaced_by_resource_id
                            .eq(Some(source_resource_id)),
                    ))
                    .execute(&mut conn)
                    .unwrap();
            }
            SuccessorCase::OverDepth => {
                let mut parent_id = source_input.resource_id.clone().unwrap();
                for index in 0..65 {
                    let child_id = format!("dr_matrix_depth_{index}");
                    publish_unrelated_para_with_lineage(
                        &app,
                        &parent_id,
                        &child_id,
                        json!({"replacesResourceId":parent_id}),
                    );
                    diesel::update(zjudss::db::schema::data_resources::table.find(&parent_id))
                        .set(
                            zjudss::db::schema::data_resources::replaced_by_resource_id
                                .eq(Some(child_id.clone())),
                        )
                        .execute(&mut app.pool.get().unwrap())
                        .unwrap();
                    parent_id = child_id;
                }
            }
            SuccessorCase::Unchanged | SuccessorCase::SourceLess => {}
        }

        let before_counts = {
            let mut conn = app.pool.get().unwrap();
            (
                zjudss::db::schema::managed_workflows::table
                    .count()
                    .get_result::<i64>(&mut conn)
                    .unwrap(),
                zjudss::db::schema::simulation_runs::table
                    .count()
                    .get_result::<i64>(&mut conn)
                    .unwrap(),
            )
        };
        let retry = app
            .state
            .managed_workflow_service
            .preview_retry(
                &user,
                source_script_id,
                RetryStudyRequest {
                    client_retry_id: format!("matrix-retry-{}", case.name()),
                },
            )
            .await;
        let rerun = app
            .state
            .run_service
            .create(
                &user,
                source_script_id,
                CreateRunRequest {
                    client_run_id: format!("matrix-rerun-{}", case.name()),
                },
            )
            .await;

        if case.rejects() {
            assert_eq!(retry.unwrap_err().code(), "configuration_stale", "{case:?}");
            assert!(matches!(rerun.unwrap_err(), RunError::ConfigurationStale));
            let mut conn = app.pool.get().unwrap();
            let after_counts = (
                zjudss::db::schema::managed_workflows::table
                    .count()
                    .get_result::<i64>(&mut conn)
                    .unwrap(),
                zjudss::db::schema::simulation_runs::table
                    .count()
                    .get_result::<i64>(&mut conn)
                    .unwrap(),
            );
            assert_eq!(after_counts, before_counts, "{case:?}");
            continue;
        }

        let preview = retry.unwrap();
        let retry_workflow = if let Some(workflow) = preview.workflow {
            workflow
        } else {
            app.state
                .managed_workflow_service
                .confirm_retry(
                    &user,
                    &preview.retry_preview_id.unwrap(),
                    ConfirmRetryRequest { confirmed: true },
                )
                .await
                .unwrap()
        };
        let retry_input = frozen_para(&app, retry_workflow.id);
        let retry_snapshot: Value = serde_json::from_str(&retry_input.snapshot_json).unwrap();
        assert_eq!(
            retry_snapshot["effective"]["userOverrides"],
            if matches!(case, SuccessorCase::LegacyMergedOverrides) {
                json!({})
            } else {
                expected_user_overrides.clone()
            },
            "{case:?}"
        );
        assert_eq!(
            retry_snapshot["effective"]["selectorOverrides"],
            if matches!(case, SuccessorCase::LegacyMergedOverrides) {
                json!({})
            } else {
                expected_selector_overrides.clone()
            },
            "{case:?}"
        );
        assert_eq!(
            retry_snapshot["effective"]["overrides"], expected_merged_overrides,
            "{case:?}"
        );
        let rerun = rerun.unwrap();
        let rerun_snapshot: Value = serde_json::from_str(&rerun.run.input_snapshot_json).unwrap();
        let rerun_para = rerun_snapshot["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["alias"] == "para")
            .unwrap();
        assert_eq!(
            rerun_para["resourceId"].as_str(),
            retry_input.resource_id.as_deref(),
            "{case:?}"
        );
        assert_eq!(
            rerun_para["artifactId"].as_i64(),
            retry_input.artifact_id.map(i64::from),
            "{case:?}"
        );
        assert_eq!(
            rerun_para["contentHash"].as_str(),
            retry_input.content_hash.as_deref(),
            "{case:?}"
        );
        assert_eq!(
            rerun_para["schemaHash"].as_str(),
            retry_input.schema_hash.as_deref(),
            "{case:?}"
        );
    }
}

#[tokio::test]
async fn unchanged_private_overlay_retry_reuses_the_effective_artifact() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
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
async fn concurrent_replay_never_consumes_uncommitted_overlay_artifacts() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
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
        .rows(&user, &para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows.into_iter().next().unwrap())
        .collect::<serde_json::Map<_, _>>();
    values.insert("tol".into(), json!(0.02));
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
    assert_eq!(max_iter.value(0), 37);
    let tol = batch
        .column(batch.schema().index_of("tol").unwrap())
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert_eq!(tol.value(0), 1e-7);

    let current_para = app
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
        .rows(&user, &current_para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    values.insert("max_iter".into(), json!(88));
    let replacement = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &current_para.id,
            &current_para.content_hash,
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
    let usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let counts_before_replay = {
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
        .preview_retry(
            &user,
            source_script_id,
            RetryStudyRequest {
                client_retry_id: "retry-changed-overlay".into(),
            },
        )
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
    assert!(replay.retry_preview_id.is_none());
    assert_eq!(replay.workflow.unwrap().id, retry.id);
    let usage_after_replay = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let mut conn = app.pool.get().unwrap();
    let counts_after_replay = (
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
    assert_eq!(counts_after_replay, counts_before_replay);
    assert_eq!(usage_after_replay.used_artifacts, usage.used_artifacts);
    assert_eq!(usage_after_replay.used_bytes, usage.used_bytes);
}

#[tokio::test]
async fn retry_rejects_same_alias_resource_without_replacement_lineage() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
    let source_resource_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::resource_id)
        .first::<Option<String>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    publish_unrelated_para_with_lineage(
        &app,
        &source_resource_id,
        "dr_unrelated_same_alias",
        json!({}),
    );

    let error = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source.final_script_resource_id.unwrap(),
            RetryStudyRequest {
                client_retry_id: "retry-unrelated-same-alias".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "configuration_stale");
}

#[tokio::test]
async fn source_less_overlay_retry_reuses_frozen_effective_input() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, true).await;
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
            source.final_script_resource_id.unwrap(),
            RetryStudyRequest {
                client_retry_id: "retry-source-less-overlay".into(),
            },
        )
        .await
        .unwrap();
    let retry = preview.workflow.unwrap();
    let retry_input = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(retry.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select((
            zjudss::db::schema::workflow_input_snapshots::resource_id,
            zjudss::db::schema::workflow_input_snapshots::artifact_id,
        ))
        .first::<(Option<String>, Option<i32>)>(&mut app.pool.get().unwrap())
        .unwrap();
    assert!(retry_input.0.is_none());
    assert_eq!(retry_input.1, Some(source_artifact_id));
}

#[tokio::test]
async fn legacy_overlay_snapshot_retries_with_merged_overrides() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
    let mut conn = app.pool.get().unwrap();
    let raw = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::snapshot_json)
        .first::<String>(&mut conn)
        .unwrap();
    let mut snapshot: Value = serde_json::from_str(&raw).unwrap();
    let effective = snapshot["effective"].as_object_mut().unwrap();
    effective.remove("userOverrides");
    effective.remove("selectorOverrides");
    diesel::sql_query("DROP TRIGGER immutable_workflow_inputs")
        .execute(&mut conn)
        .unwrap();
    diesel::update(
        zjudss::db::schema::workflow_input_snapshots::table
            .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
            .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para")),
    )
    .set(zjudss::db::schema::workflow_input_snapshots::snapshot_json.eq(snapshot.to_string()))
    .execute(&mut conn)
    .unwrap();
    drop(conn);

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
        .rows(&user, &para.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows.into_iter().next().unwrap())
        .collect::<serde_json::Map<_, _>>();
    values.insert("max_iter".into(), json!(99));
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
            source.final_script_resource_id.unwrap(),
            RetryStudyRequest {
                client_retry_id: "retry-legacy-overlay".into(),
            },
        )
        .await
        .unwrap();
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
    let max_iter = batch
        .column(batch.schema().index_of("max_iter").unwrap())
        .as_any()
        .downcast_ref::<UInt32Array>()
        .unwrap();
    assert_eq!(max_iter.value(0), 37);
}

#[tokio::test]
async fn retry_rejects_cyclic_replacement_lineage() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
    let source_resource_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::resource_id)
        .first::<Option<String>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    publish_unrelated_para_with_lineage(
        &app,
        &source_resource_id,
        "dr_cyclic_para",
        json!({"replacesResourceId":source_resource_id}),
    );
    let mut conn = app.pool.get().unwrap();
    diesel::update(zjudss::db::schema::data_resources::table.find(&source_resource_id))
        .set(
            zjudss::db::schema::data_resources::replaced_by_resource_id
                .eq(Some("dr_cyclic_para".to_string())),
        )
        .execute(&mut conn)
        .unwrap();
    diesel::update(zjudss::db::schema::data_resources::table.find("dr_cyclic_para"))
        .set((
            zjudss::db::schema::data_resources::status.eq("replaced"),
            zjudss::db::schema::data_resources::replaced_by_resource_id
                .eq(Some(source_resource_id.clone())),
        ))
        .execute(&mut conn)
        .unwrap();
    drop(conn);

    let error = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source.final_script_resource_id.unwrap(),
            RetryStudyRequest {
                client_retry_id: "retry-cyclic-lineage".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "configuration_stale");
}

#[tokio::test]
async fn retry_rejects_replacement_lineage_deeper_than_limit() {
    let app = common::TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let source = failed_overlay_source(&app, &user, false).await;
    let source_resource_id = zjudss::db::schema::workflow_input_snapshots::table
        .filter(zjudss::db::schema::workflow_input_snapshots::workflow_id.eq(source.id))
        .filter(zjudss::db::schema::workflow_input_snapshots::input_name.eq("para"))
        .select(zjudss::db::schema::workflow_input_snapshots::resource_id)
        .first::<Option<String>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    let mut parent_id = source_resource_id;
    for index in 0..65 {
        let child_id = format!("dr_deep_para_{index}");
        publish_unrelated_para_with_lineage(
            &app,
            &parent_id,
            &child_id,
            json!({"replacesResourceId":parent_id.clone()}),
        );
        diesel::update(zjudss::db::schema::data_resources::table.find(&parent_id))
            .set(
                zjudss::db::schema::data_resources::replaced_by_resource_id
                    .eq(Some(child_id.clone())),
            )
            .execute(&mut app.pool.get().unwrap())
            .unwrap();
        parent_id = child_id;
    }

    let error = app
        .state
        .managed_workflow_service
        .preview_retry(
            &user,
            source.final_script_resource_id.unwrap(),
            RetryStudyRequest {
                client_retry_id: "retry-over-depth-lineage".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "configuration_stale");
}

#[tokio::test]
async fn unchanged_failed_agent_script_starts_a_derived_retry_with_a_fresh_budget() {
    let author = Arc::new(ScriptedAuthor::default());
    let engine = Arc::new(ScenarioEngine::default());
    let app = common::TestApp::builder()
        .engine(engine.clone())
        .agent_factory(Arc::new(ScriptedAgentFactory::default()))
        .script_author(author.clone())
        .failure_analyzer(author.clone())
        .catalog_mode(zjudss::testing::E2eCatalogMode::Minimal)
        .spawn()
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
    let app = common::TestApp::builder()
        .engine(engine.clone())
        .agent_factory(Arc::new(ScriptedAgentFactory::default()))
        .script_author(author.clone())
        .failure_analyzer(author.clone())
        .catalog_mode(zjudss::testing::E2eCatalogMode::Minimal)
        .spawn()
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
    let malformed = app
        .state
        .managed_workflow_service
        .confirm_retry(
            &user,
            &token.to_ascii_uppercase(),
            ConfirmRetryRequest { confirmed: true },
        )
        .await
        .unwrap_err();
    assert_eq!(malformed.code(), "invalid_retry_preview");
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
