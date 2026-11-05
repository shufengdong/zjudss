impl ManagedWorkflowRepo {
    pub async fn record_candidate(
        &self,
        user: &UserId,
        workflow_id: i32,
        candidate: NewWorkflowCandidate,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned_on(conn, &user, workflow_id)?;
                if !matches!(
                    current.workflow.status.as_str(),
                    "queued" | "generating" | "repairing"
                ) {
                    return Err(ManagedWorkflowRepoError::TerminalConflict);
                }
                if candidate.phase != "generation" && candidate.phase != "repair" {
                    return Err(ManagedWorkflowRepoError::Repository(
                        RepoError::InvalidState("attempt_phase_invalid".into()),
                    ));
                }
                if let Some(parent_id) = candidate.parent_attempt_id {
                    let valid_parent = current
                        .attempts
                        .last()
                        .is_some_and(|attempt| attempt.id == parent_id);
                    if !valid_parent {
                        return Err(ManagedWorkflowRepoError::Repository(
                            RepoError::InvalidState("attempt_parent_invalid".into()),
                        ));
                    }
                } else if !current.attempts.is_empty() {
                    return Err(ManagedWorkflowRepoError::Repository(
                        RepoError::InvalidState("attempt_parent_required".into()),
                    ));
                }
                let attempt_number = i32::try_from(current.attempts.len() + 1)
                    .map_err(|_| RepoError::InvalidState("attempt_limit".into()))?;
                diesel::insert_into(schema::workflow_attempts::table)
                    .values((
                        schema::workflow_attempts::workflow_id.eq(workflow_id),
                        schema::workflow_attempts::attempt_number.eq(attempt_number),
                        schema::workflow_attempts::parent_attempt_id
                            .eq(candidate.parent_attempt_id),
                        schema::workflow_attempts::phase.eq(&candidate.phase),
                        schema::workflow_attempts::script_artifact_id
                            .eq(Some(candidate.artifact_id)),
                        schema::workflow_attempts::script_content_hash
                            .eq(Some(&candidate.content_hash)),
                        schema::workflow_attempts::status.eq("candidate"),
                        schema::workflow_attempts::agent_bundle_version
                            .eq(Some(candidate.agent_bundle_version)),
                        schema::workflow_attempts::agent_skill_refs_json
                            .eq(candidate.agent_skill_refs_json),
                        schema::workflow_attempts::agent_model.eq(candidate.agent_model),
                        schema::workflow_attempts::engine_snapshot_id
                            .eq(Some(candidate.engine_snapshot_id)),
                        schema::workflow_attempts::created_at_ms.eq(candidate.now_ms),
                    ))
                    .execute(conn)?;
                diesel::update(schema::managed_workflows::table.find(workflow_id))
                    .set((
                        schema::managed_workflows::status.eq(if candidate.phase == "repair" {
                            "repairing"
                        } else {
                            "generating"
                        }),
                        schema::managed_workflows::corrections_used
                            .eq(schema::managed_workflows::corrections_used
                                + if candidate.phase == "repair" { 1 } else { 0 }),
                        schema::managed_workflows::updated_at_ms.eq(candidate.now_ms),
                    ))
                    .execute(conn)?;
                update_progress_message(
                    conn,
                    workflow_id,
                    if candidate.phase == "repair" {
                        "repairing"
                    } else {
                        "generating"
                    },
                    candidate.now_ms,
                )?;
                load_owned_on(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn update_attempt(
        &self,
        user: &UserId,
        workflow_id: i32,
        attempt_id: i32,
        update: WorkflowAttemptUpdate,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned_on(conn, &user, workflow_id)?;
                if !matches!(
                    current.workflow.status.as_str(),
                    "queued" | "generating" | "executing" | "repairing"
                ) || current.attempts.last().map(|attempt| attempt.id) != Some(attempt_id)
                {
                    return Err(ManagedWorkflowRepoError::TerminalConflict);
                }
                let existing_attempt = current.attempts.last().expect("last attempt checked");
                let changed = diesel::update(
                    schema::workflow_attempts::table
                        .find(attempt_id)
                        .filter(schema::workflow_attempts::workflow_id.eq(workflow_id)),
                )
                .set((
                    schema::workflow_attempts::status.eq(&update.status),
                    schema::workflow_attempts::exit_code.eq(update.exit_code),
                    schema::workflow_attempts::stderr_audit_artifact_id
                        .eq(update.stderr_audit_artifact_id),
                    schema::workflow_attempts::sanitized_stderr.eq(update.sanitized_stderr),
                    schema::workflow_attempts::stderr_full_hash.eq(update.stderr_full_hash),
                    schema::workflow_attempts::stderr_retained_hash.eq(update.stderr_retained_hash),
                    schema::workflow_attempts::stderr_sent_hash.eq(update.stderr_sent_hash),
                    schema::workflow_attempts::stderr_total_bytes.eq(update.stderr_total_bytes),
                    schema::workflow_attempts::stderr_sent_bytes.eq(update.stderr_sent_bytes),
                    schema::workflow_attempts::stderr_truncated.eq(update.stderr_truncated),
                    schema::workflow_attempts::result_diagnostics_json.eq(update.diagnostic_json),
                    schema::workflow_attempts::started_at_ms.eq(if update.started {
                        Some(update.now_ms)
                    } else {
                        existing_attempt.started_at_ms
                    }),
                    schema::workflow_attempts::finished_at_ms.eq(if update.finished {
                        Some(update.now_ms)
                    } else {
                        existing_attempt.finished_at_ms
                    }),
                ))
                .execute(conn)?;
                if changed != 1 {
                    return Err(ManagedWorkflowRepoError::Repository(RepoError::NotFound));
                }
                diesel::update(schema::managed_workflows::table.find(workflow_id))
                    .set((
                        schema::managed_workflows::status.eq(&update.workflow_status),
                        schema::managed_workflows::updated_at_ms.eq(update.now_ms),
                    ))
                    .execute(conn)?;
                diesel::update(schema::simulation_runs::table.find(current.run.id))
                    .set((
                        schema::simulation_runs::status.eq(
                            if update.workflow_status == "executing" {
                                "running"
                            } else {
                                "queued"
                            },
                        ),
                        schema::simulation_runs::stage.eq(&update.workflow_status),
                        schema::simulation_runs::started_at_ms.eq(if update.started {
                            Some(update.now_ms)
                        } else {
                            current.run.started_at_ms
                        }),
                    ))
                    .execute(conn)?;
                update_progress_message(conn, workflow_id, &update.workflow_status, update.now_ms)?;
                load_owned_on(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn finalize_with_script(
        &self,
        user: &UserId,
        workflow_id: i32,
        attempt_id: i32,
        succeeded: bool,
        error_json: Option<String>,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        self.finalize_with_outputs(
            user,
            workflow_id,
            attempt_id,
            succeeded,
            error_json,
            Vec::new(),
            now_ms,
        )
        .await
    }

    pub async fn fail_without_candidate(
        &self,
        user: &UserId,
        workflow_id: i32,
        error_json: String,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned_on(conn, &user, workflow_id)?;
                if !current.attempts.is_empty() {
                    return Err(ManagedWorkflowRepoError::TerminalConflict);
                }
                terminalize(
                    conn,
                    &user,
                    &current,
                    None,
                    "failed",
                    "failed",
                    Some(error_json),
                    now_ms,
                    &[],
                )?;
                load_owned_on(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_with_outputs(
        &self,
        user: &UserId,
        workflow_id: i32,
        attempt_id: i32,
        succeeded: bool,
        error_json: Option<String>,
        datasets: Vec<NewRunDataset>,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned_on(conn, &user, workflow_id)?;
                if matches!(current.workflow.status.as_str(), "succeeded" | "failed") {
                    if current.workflow.final_script_resource_id.is_some() {
                        return Ok(current);
                    }
                    return Err(ManagedWorkflowRepoError::TerminalConflict);
                }
                if !matches!(
                    current.workflow.status.as_str(),
                    "queued" | "generating" | "executing" | "repairing"
                ) {
                    return Err(ManagedWorkflowRepoError::TerminalConflict);
                }
                let terminal = if succeeded { "succeeded" } else { "failed" };
                terminalize(
                    conn,
                    &user,
                    &current,
                    Some(attempt_id),
                    terminal,
                    terminal,
                    error_json,
                    now_ms,
                    &datasets,
                )?;
                load_owned_on(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_with_artifact_outputs<P>(
        &self,
        user: &UserId,
        storage_scope_id: i32,
        workflow_id: i32,
        attempt_id: i32,
        drafts: Vec<ArtifactDraft>,
        pending: Vec<PendingRunDataset>,
        publish: P,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError>
    where
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
    {
        if drafts.len() != pending.len() {
            return Err(ManagedWorkflowRepoError::Repository(
                RepoError::InvalidState("result_artifact_count_mismatch".into()),
            ));
        }
        let owner = user.0.clone();
        ArtifactRepo::new(self.pool.clone())
            .commit_batch_with_publish(storage_scope_id, drafts, publish, move |conn, artifacts| {
                let current = load_owned_on(conn, &owner, workflow_id)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
                if matches!(current.workflow.status.as_str(), "succeeded" | "failed") {
                    return Err(RepoError::InvalidState("workflow_terminal_conflict".into()));
                }
                let datasets = pending
                    .iter()
                    .zip(artifacts)
                    .map(|(dataset, artifact)| NewRunDataset {
                        artifact_id: artifact.id,
                        result_name: dataset.result_name.clone(),
                        schema_json: dataset.schema_json.clone(),
                        schema_hash: dataset.schema_hash.clone(),
                        row_count: dataset.row_count,
                        size_bytes: dataset.size_bytes,
                        created_at_ms: dataset.created_at_ms,
                    })
                    .collect::<Vec<_>>();
                terminalize(
                    conn,
                    &owner,
                    &current,
                    Some(attempt_id),
                    "succeeded",
                    "succeeded",
                    None,
                    now_ms,
                    &datasets,
                )
                .map_err(|error| RepoError::InvalidState(error.to_string()))
            })
            .await
            .map_err(ManagedWorkflowRepoError::Repository)?;
        self.find(user, workflow_id).await
    }
}

#[allow(clippy::too_many_arguments)]
fn terminalize(
    conn: &mut SqliteConnection,
    user: &str,
    current: &ManagedWorkflowAggregate,
    attempt_id: Option<i32>,
    workflow_status: &str,
    run_stage: &str,
    error_json: Option<String>,
    now_ms: i64,
    datasets: &[NewRunDataset],
) -> Result<(), ManagedWorkflowRepoError> {
    let workflow_id = current.workflow.id;
    let selected = if let Some(attempt_id) = attempt_id {
        Some(
            current
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .ok_or_else(|| ManagedWorkflowRepoError::Repository(RepoError::NotFound))?,
        )
    } else {
        current
            .attempts
            .iter()
            .rev()
            .find(|attempt| attempt.script_artifact_id.is_some())
    };
    let mut script_id = current.workflow.final_script_resource_id;
    if script_id.is_none()
        && let Some(attempt) = selected
    {
        let artifact_id = attempt.script_artifact_id.ok_or_else(|| {
            ManagedWorkflowRepoError::Repository(RepoError::InvalidState(
                "candidate_script_missing".into(),
            ))
        })?;
        let content_hash = attempt.script_content_hash.clone().ok_or_else(|| {
            ManagedWorkflowRepoError::Repository(RepoError::InvalidState(
                "candidate_script_missing".into(),
            ))
        })?;
        let source_script_resource_id = match current.workflow.source_workflow_id {
            Some(source_id) => schema::managed_workflows::table
                .find(source_id)
                .select(schema::managed_workflows::final_script_resource_id)
                .first::<Option<i32>>(conn)?,
            None => None,
        };
        diesel::insert_into(schema::script_resources::table)
            .values((
                schema::script_resources::session_id.eq(current.workflow.session_id),
                schema::script_resources::artifact_id.eq(artifact_id),
                schema::script_resources::name.eq(format!("managed-workflow-{workflow_id}.rss")),
                schema::script_resources::source_kind.eq("agent"),
                schema::script_resources::content_hash.eq(&content_hash),
                schema::script_resources::validation_status.eq("valid"),
                schema::script_resources::required_aliases_json.eq("[]"),
                schema::script_resources::outputs_json.eq("[]"),
                schema::script_resources::diagnostics_json.eq("{}"),
                schema::script_resources::agent_bundle_version
                    .eq(attempt.agent_bundle_version.clone()),
                schema::script_resources::agent_skill_refs_json.eq(&attempt.agent_skill_refs_json),
                schema::script_resources::managed_workflow_id.eq(Some(workflow_id)),
                schema::script_resources::source_script_resource_id.eq(source_script_resource_id),
                schema::script_resources::frozen_contract_json
                    .eq(current.workflow.locked_contract_json.clone()),
                schema::script_resources::engine_snapshot_id
                    .eq(current.workflow.engine_snapshot_id.clone()),
                schema::script_resources::facade_contract_hash
                    .eq(current.workflow.facade_contract_hash.clone()),
                schema::script_resources::created_at_ms.eq(now_ms),
            ))
            .execute(conn)?;
        let published_id = last_id(conn)?;
        retain_artifact(conn, user, artifact_id, 2)?;
        let script_payload = json!({
            "script": {
                "id": published_id.to_string(),
                "name": format!("managed-workflow-{workflow_id}.rss"),
                "contentHash": content_hash,
                "managedWorkflowId": workflow_id.to_string(),
            }
        })
        .to_string();
        diesel::insert_into(schema::messages::table)
            .values((
                schema::messages::session_id.eq(current.workflow.session_id),
                schema::messages::run_id.eq(Some(current.run.id)),
                schema::messages::revision.eq(1),
                schema::messages::role.eq("tool"),
                schema::messages::kind.eq("script"),
                schema::messages::content.eq(format!("脚本：managed-workflow-{workflow_id}.rss")),
                schema::messages::payload_json.eq(Some(script_payload)),
                schema::messages::created_at_ms.eq(now_ms),
            ))
            .execute(conn)?;
        let script_message_id = last_id(conn)?;
        diesel::update(schema::simulation_runs::table.find(current.run.id))
            .set((
                schema::simulation_runs::script_resource_id.eq(Some(published_id)),
                schema::simulation_runs::script_artifact_id.eq(Some(artifact_id)),
                schema::simulation_runs::script_content_hash.eq(Some(content_hash)),
                schema::simulation_runs::script_message_id.eq(Some(script_message_id)),
            ))
            .execute(conn)?;
        script_id = Some(published_id);
    } else if attempt_id.is_some() && script_id.is_none() {
        return Err(ManagedWorkflowRepoError::Repository(
            RepoError::InvalidState("candidate_script_missing".into()),
        ));
    }
    let attempt_status = if workflow_status == "succeeded" {
        "succeeded"
    } else if selected.is_some_and(|attempt| {
        matches!(
            attempt.status.as_str(),
            "contract_rejected" | "engine_failed" | "result_rejected"
        )
    }) {
        selected.expect("selected checked").status.as_str()
    } else {
        "engine_failed"
    };
    let attempt_error_json = if workflow_status != "succeeded"
        && selected.is_some_and(|attempt| attempt.result_diagnostics_json.is_some())
    {
        selected.and_then(|attempt| attempt.result_diagnostics_json.clone())
    } else {
        error_json.clone()
    };
    if let Some(attempt_id) = attempt_id {
        diesel::update(schema::workflow_attempts::table.find(attempt_id))
            .set((
                schema::workflow_attempts::status.eq(attempt_status),
                schema::workflow_attempts::exit_code.eq(if workflow_status == "succeeded" {
                    Some(0)
                } else {
                    selected.and_then(|attempt| attempt.exit_code)
                }),
                schema::workflow_attempts::result_diagnostics_json.eq(attempt_error_json.clone()),
                schema::workflow_attempts::finished_at_ms.eq(Some(now_ms)),
            ))
            .execute(conn)?;
    } else {
        diesel::update(
            schema::workflow_attempts::table
                .filter(schema::workflow_attempts::workflow_id.eq(workflow_id))
                .filter(schema::workflow_attempts::status.eq_any(["candidate", "executing"])),
        )
        .set((
            schema::workflow_attempts::status.eq(attempt_status),
            schema::workflow_attempts::result_diagnostics_json.eq(error_json.clone()),
            schema::workflow_attempts::finished_at_ms.eq(Some(now_ms)),
        ))
        .execute(conn)?;
    }
    let run_status = if workflow_status == "succeeded" {
        "succeeded"
    } else {
        "failed"
    };
    if workflow_status == "succeeded" {
        for dataset in datasets {
            diesel::insert_into(schema::result_datasets::table)
                .values(crate::db::models::NewResultDataset {
                    run_id: Some(current.run.id),
                    analysis_run_id: None,
                    artifact_id: Some(dataset.artifact_id),
                    result_name: dataset.result_name.clone(),
                    schema_json: dataset.schema_json.clone(),
                    schema_hash: dataset.schema_hash.clone(),
                    row_count: dataset.row_count,
                    size_bytes: dataset.size_bytes,
                    created_at_ms: dataset.created_at_ms,
                })
                .execute(conn)?;
        }
    } else if !datasets.is_empty() {
        return Err(ManagedWorkflowRepoError::Repository(
            RepoError::InvalidState("failed_workflow_has_results".into()),
        ));
    }
    diesel::update(schema::simulation_runs::table.find(current.run.id))
        .set((
            schema::simulation_runs::status.eq(run_status),
            schema::simulation_runs::stage.eq(run_stage),
            schema::simulation_runs::error_json.eq(error_json.clone()),
            schema::simulation_runs::exit_code.eq(if workflow_status == "succeeded" {
                Some(0)
            } else {
                selected.and_then(|attempt| attempt.exit_code)
            }),
            schema::simulation_runs::finished_at_ms.eq(Some(now_ms)),
        ))
        .execute(conn)?;
    let changed = diesel::update(schema::managed_workflows::table.find(workflow_id).filter(
        schema::managed_workflows::status.eq_any([
            "queued",
            "generating",
            "executing",
            "repairing",
        ]),
    ))
    .set((
        schema::managed_workflows::status.eq(workflow_status),
        schema::managed_workflows::final_script_resource_id.eq(script_id),
        schema::managed_workflows::error_json.eq(error_json),
        schema::managed_workflows::updated_at_ms.eq(now_ms),
        schema::managed_workflows::finished_at_ms.eq(Some(now_ms)),
    ))
    .execute(conn)?;
    if changed != 1 {
        return Err(ManagedWorkflowRepoError::TerminalConflict);
    }
    update_progress_message(conn, workflow_id, workflow_status, now_ms)
}

fn update_progress_message(
    conn: &mut SqliteConnection,
    workflow_id: i32,
    state: &str,
    now_ms: i64,
) -> Result<(), ManagedWorkflowRepoError> {
    let workflow = schema::managed_workflows::table
        .find(workflow_id)
        .first::<ManagedWorkflowRecord>(conn)?;
    let Some(message_id) = workflow.progress_message_id else {
        return Ok(());
    };
    let attempts = schema::workflow_attempts::table
        .filter(schema::workflow_attempts::workflow_id.eq(workflow_id))
        .order(schema::workflow_attempts::attempt_number.asc())
        .load::<WorkflowAttemptRecord>(conn)?
        .into_iter()
        .map(|attempt| {
            json!({
                "executionIndex": attempt.attempt_number,
                "repairIndex": if attempt.phase == "repair" { attempt.attempt_number.saturating_sub(1) } else { 0 },
                "state": attempt.status,
                "elapsedMs": 0,
                "failureCode": attempt.result_diagnostics_json.as_deref().and_then(|raw|serde_json::from_str::<serde_json::Value>(raw).ok()).and_then(|value|value.get("code").and_then(|code|code.as_str()).map(str::to_string)),
            })
        })
        .collect::<Vec<_>>();
    let output_dataset_ids = workflow
        .final_run_id
        .map(|run_id| {
            schema::result_datasets::table
                .filter(schema::result_datasets::run_id.eq(run_id))
                .filter(schema::result_datasets::deleted_at_ms.is_null())
                .order(schema::result_datasets::id.asc())
                .select(schema::result_datasets::id)
                .load::<i32>(conn)
        })
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let payload = json!({
        "workflowId": workflow_id.to_string(),
        "state": state,
        "executionIndex": 0,
        "executionMax": workflow.correction_budget + 1,
        "repairIndex": workflow.corrections_used,
        "repairMax": workflow.correction_budget,
        "elapsedMs": 0,
        "attempts": attempts,
        "scriptId": workflow.final_script_resource_id.map(|id|id.to_string()),
        "outputDatasetIds": output_dataset_ids,
        "failureCode": workflow.error_json.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()).and_then(|value| value.get("code").and_then(|code|code.as_str()).map(str::to_string)),
    })
    .to_string();
    diesel::update(schema::messages::table.find(message_id))
        .set((
            schema::messages::revision.eq(schema::messages::revision + 1),
            schema::messages::content.eq(format!("研究工作流：{state}")),
            schema::messages::payload_json.eq(Some(payload)),
            schema::messages::created_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    Ok(())
}

fn retain_artifact(
    conn: &mut SqliteConnection,
    user: &str,
    artifact_id: i32,
    count: i64,
) -> Result<(), ManagedWorkflowRepoError> {
    let scope_id = super::artifact_refs::storage_scope_id_for_user(conn, &UserId(user.to_owned()))?;
    super::artifact_refs::retain_ref_count(conn, scope_id, artifact_id, count)?;
    Ok(())
}

