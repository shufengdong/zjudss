impl ManagedWorkflowRepo {
    pub async fn find_confirmation_replay(
        &self,
        user: &UserId,
        configuration_id: i32,
        expected_revision: i64,
        client_operation_id: &str,
    ) -> Result<Option<ManagedWorkflowAggregate>, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let operation = client_operation_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            let session_id = schema::study_configurations::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::study_configurations::session_id)),
                )
                .filter(schema::study_configurations::id.eq(configuration_id))
                .filter(schema::sessions::user_id.eq(&user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::study_configurations::session_id)
                .first::<i32>(&mut conn)
                .optional()?
                .ok_or(ManagedWorkflowRepoError::Repository(RepoError::NotFound))?;
            let existing = schema::managed_workflows::table
                .filter(schema::managed_workflows::session_id.eq(session_id))
                .filter(schema::managed_workflows::client_operation_id.eq(Some(operation)))
                .first::<ManagedWorkflowRecord>(&mut conn)
                .optional()?;
            let Some(existing) = existing else {
                return Ok(None);
            };
            if existing.configuration_id != Some(configuration_id)
                || existing.configuration_revision != Some(expected_revision)
            {
                return Err(ManagedWorkflowRepoError::IdempotencyConflict);
            }
            load_owned_on(&mut conn, &user, existing.id).map(Some)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_and_start(
        &self,
        user: &UserId,
        command: ManagedWorkflowStartCommand,
        overlays: Vec<WorkflowStructOverlayMaterial>,
    ) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError> {
        let inputs = overlays
            .into_iter()
            .map(workflow_overlay_input_from_material)
            .collect();
        self.confirm_and_start_with_overlay_artifacts(user, command, inputs, Vec::new(), |_, _| {
            Ok(())
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_and_start_with_overlay_artifacts<P>(
        &self,
        user: &UserId,
        command: ManagedWorkflowStartCommand,
        inputs: Vec<WorkflowOverlayInput>,
        drafts: Vec<ArtifactDraft>,
        publish: P,
    ) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError>
    where
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
    {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let mut frozen_inputs: FrozenStartInputs =
            serde_json::from_str(&command.frozen_inputs_json)
                .map_err(|error| RepoError::Storage(error.to_string()))?;
        let struct_overlays: Vec<StartStructOverlay> =
            serde_json::from_str(&command.struct_overlays_json)
                .map_err(|error| RepoError::Storage(error.to_string()))?;
        let scope_id = owned_scope_id(&pool, &user).await?;
        ArtifactRepo::new(pool)
            .commit_batch_with_publish(scope_id, drafts, publish, move |conn, artifacts| {
                let committed = commit_overlay_inputs_on(conn, scope_id, inputs, artifacts)
                    .map_err(configuration_artifact_error)?;
                let adopted = committed
                    .iter()
                    .filter(|input| input.adopts_batch_link)
                    .map(|input| input.artifact_id)
                    .collect::<Vec<_>>();
                let overlays = committed
                    .into_iter()
                    .map(WorkflowStructOverlayMaterial::from)
                    .collect::<Vec<_>>();
                let result = (|| {
                    let configuration =
                        schema::study_configurations::table
                            .inner_join(schema::sessions::table.on(
                                schema::sessions::id.eq(schema::study_configurations::session_id),
                            ))
                            .filter(schema::study_configurations::id.eq(command.configuration_id))
                            .filter(schema::sessions::user_id.eq(&user))
                            .filter(schema::sessions::deletion_status.eq("live"))
                            .select(schema::study_configurations::all_columns)
                            .first::<crate::db::models::StudyConfigurationRecord>(conn)
                            .optional()?
                            .ok_or(ManagedWorkflowRepoError::Repository(RepoError::NotFound))?;

                    if let Some(existing) = schema::managed_workflows::table
                        .filter(schema::managed_workflows::session_id.eq(configuration.session_id))
                        .filter(
                            schema::managed_workflows::client_operation_id
                                .eq(Some(command.client_operation_id.clone())),
                        )
                        .first::<ManagedWorkflowRecord>(conn)
                        .optional()?
                    {
                        if existing.configuration_id != Some(command.configuration_id)
                            || existing.configuration_revision
                                != Some(command.configuration_revision)
                        {
                            return Err(ManagedWorkflowRepoError::IdempotencyConflict);
                        }
                        return load_owned_on(conn, &user, existing.id).map(|aggregate| {
                            ConfirmStartOutcome {
                                aggregate,
                                created: false,
                            }
                        });
                    }

                    if configuration.session_id != command.session_id
                        || configuration.revision != command.configuration_revision
                        || configuration.status != "ready_to_confirm"
                        || configuration.engine_snapshot_id != command.engine_snapshot_id
                        || configuration.facade_hash != command.facade_contract_hash
                    {
                        return Err(ManagedWorkflowRepoError::ConfigurationStale);
                    }
                    if struct_overlays.len() != overlays.len() {
                        return Err(ManagedWorkflowRepoError::ConfigurationStale);
                    }
                    if let Some(active_id) = schema::managed_workflows::table
                        .filter(schema::managed_workflows::session_id.eq(configuration.session_id))
                        .filter(schema::managed_workflows::status.eq_any([
                            "queued",
                            "generating",
                            "executing",
                            "repairing",
                        ]))
                        .select(schema::managed_workflows::id)
                        .first::<i32>(conn)
                        .optional()?
                    {
                        return Err(ManagedWorkflowRepoError::WorkflowInProgress(active_id));
                    }

                    validate_active_model(conn, &user, configuration.session_id, &frozen_inputs)?;

                    let locked_json = command.locked_contract_json.clone();
                    diesel::insert_into(schema::managed_workflows::table)
                        .values((
                            schema::managed_workflows::session_id.eq(configuration.session_id),
                            schema::managed_workflows::configuration_id.eq(Some(configuration.id)),
                            schema::managed_workflows::configuration_revision
                                .eq(Some(configuration.revision)),
                            schema::managed_workflows::source_workflow_id.eq(Option::<i32>::None),
                            schema::managed_workflows::client_operation_id
                                .eq(Some(command.client_operation_id.clone())),
                            schema::managed_workflows::status.eq("queued"),
                            schema::managed_workflows::intent_json.eq(&configuration.intent_json),
                            schema::managed_workflows::locked_contract_json.eq(Some(&locked_json)),
                            schema::managed_workflows::input_diff_json.eq(Option::<String>::None),
                            schema::managed_workflows::engine_snapshot_id
                                .eq(Some(&command.engine_snapshot_id)),
                            schema::managed_workflows::facade_contract_hash
                                .eq(Some(&command.facade_contract_hash)),
                            schema::managed_workflows::correction_budget
                                .eq(command.correction_budget),
                            schema::managed_workflows::corrections_used.eq(0),
                            schema::managed_workflows::created_at_ms.eq(command.created_at_ms),
                            schema::managed_workflows::updated_at_ms.eq(command.created_at_ms),
                        ))
                        .execute(conn)?;
                    let workflow_id = last_id(conn)?;

                    freeze_inputs(
                        conn,
                        &user,
                        workflow_id,
                        &mut frozen_inputs,
                        &struct_overlays,
                        &overlays,
                        command.created_at_ms,
                    )?;
                    let inputs_json = serde_json::to_string(&frozen_inputs)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    let engine_contract_version = schema::engine_resource_snapshots::table
                        .find(&command.engine_snapshot_id)
                        .select(schema::engine_resource_snapshots::engine_contract_version)
                        .first::<String>(conn)?;
                    diesel::insert_into(schema::simulation_runs::table)
                        .values((
                            schema::simulation_runs::session_id.eq(configuration.session_id),
                            schema::simulation_runs::model_version_id.eq(frozen_inputs
                                .model
                                .as_ref()
                                .map(|model| model.model_version_id)),
                            schema::simulation_runs::model_content_hash.eq(frozen_inputs
                                .model
                                .as_ref()
                                .map(|model| model.content_hash.clone())),
                            schema::simulation_runs::script_resource_id.eq(Option::<i32>::None),
                            schema::simulation_runs::script_artifact_id.eq(Option::<i32>::None),
                            schema::simulation_runs::script_content_hash.eq(Option::<String>::None),
                            schema::simulation_runs::client_run_id
                                .eq(format!("managed:{workflow_id}")),
                            schema::simulation_runs::status.eq("queued"),
                            schema::simulation_runs::stage.eq("generating"),
                            schema::simulation_runs::engine_contract_version
                                .eq(engine_contract_version),
                            schema::simulation_runs::agent_bundle_version
                                .eq(Some(command.agent_bundle_version.clone())),
                            schema::simulation_runs::agent_skill_refs_json
                                .eq(command.agent_skill_refs_json.clone()),
                            schema::simulation_runs::managed_workflow_id.eq(Some(workflow_id)),
                            schema::simulation_runs::run_kind.eq("managed"),
                            schema::simulation_runs::input_snapshot_json.eq(inputs_json),
                            schema::simulation_runs::result_contract_json
                                .eq(Some(command.result_contract_json.clone())),
                            schema::simulation_runs::created_at_ms.eq(command.created_at_ms),
                        ))
                        .execute(conn)?;
                    let run_id = last_id(conn)?;
                    let payload = json!({
                        "workflowId": workflow_id.to_string(),
                        "state": "queued",
                        "executionIndex": 0,
                        "executionMax": command.correction_budget + 1,
                        "repairIndex": 0,
                        "repairMax": command.correction_budget,
                        "elapsedMs": 0,
                        "attempts": [],
                        "outputDatasetIds": [],
                    })
                    .to_string();
                    diesel::insert_into(schema::messages::table)
                        .values((
                            schema::messages::session_id.eq(configuration.session_id),
                            schema::messages::run_id.eq(Option::<i32>::None),
                            schema::messages::operation_id.eq(Some(workflow_id.to_string())),
                            schema::messages::revision.eq(1),
                            schema::messages::role.eq("tool"),
                            schema::messages::kind.eq("workflow-progress"),
                            schema::messages::content.eq("研究工作流：queued"),
                            schema::messages::payload_json.eq(Some(payload)),
                            schema::messages::created_at_ms.eq(command.created_at_ms),
                        ))
                        .execute(conn)?;
                    let progress_message_id = last_id(conn)?;
                    diesel::update(schema::managed_workflows::table.find(workflow_id))
                        .set((
                            schema::managed_workflows::progress_message_id
                                .eq(Some(progress_message_id)),
                            schema::managed_workflows::final_run_id.eq(Some(run_id)),
                        ))
                        .execute(conn)?;
                    let updated = diesel::update(
                        schema::study_configurations::table
                            .find(configuration.id)
                            .filter(
                                schema::study_configurations::revision.eq(configuration.revision),
                            )
                            .filter(schema::study_configurations::status.eq("ready_to_confirm")),
                    )
                    .set((
                        schema::study_configurations::status.eq("confirmed"),
                        schema::study_configurations::confirmed_at_ms
                            .eq(Some(command.created_at_ms)),
                        schema::study_configurations::updated_at_ms.eq(command.created_at_ms),
                    ))
                    .execute(conn)?;
                    if updated != 1 {
                        return Err(ManagedWorkflowRepoError::ConfigurationStale);
                    }
                    load_owned_on(conn, &user, workflow_id).map(|aggregate| ConfirmStartOutcome {
                        aggregate,
                        created: true,
                    })
                })();
                if result.is_ok() && !adopted.is_empty() {
                    super::artifact_refs::release_refs(
                        conn,
                        scope_id,
                        adopted,
                        command.created_at_ms,
                    )?;
                }
                result
            })
            .await
    }
}

fn freeze_inputs(
    conn: &mut SqliteConnection,
    user: &str,
    workflow_id: i32,
    inputs: &mut FrozenStartInputs,
    struct_overlays: &[StartStructOverlay],
    overlays: &[WorkflowStructOverlayMaterial],
    now_ms: i64,
) -> Result<(), ManagedWorkflowRepoError> {
    for prepared_overlay in struct_overlays {
        let mut matching = overlays
            .iter()
            .filter(|overlay| overlay.input_id == prepared_overlay.input_id);
        let Some(material) = matching.next() else {
            return Err(ManagedWorkflowRepoError::ConfigurationStale);
        };
        if matching.next().is_some()
            || material.source_resource_id != prepared_overlay.source_resource_id
        {
            return Err(ManagedWorkflowRepoError::ConfigurationStale);
        }
    }
    if let Some(model) = &inputs.model {
        #[derive(QueryableByName)]
        struct ModelFrame {
            #[diesel(sql_type = diesel::sql_types::Text)]
            table_name: String,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            artifact_id: i32,
            #[diesel(sql_type = diesel::sql_types::Text)]
            content_hash: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            schema_json: String,
        }
        for table in &model.tables {
            let frame = diesel::sql_query(
                "SELECT table_name,artifact_id,content_hash,schema_json FROM model_dataframes WHERE version_id=?1 AND table_name=?2",
            )
            .bind::<diesel::sql_types::Integer, _>(model.model_version_id)
            .bind::<diesel::sql_types::Text, _>(table)
            .get_result::<ModelFrame>(conn)
            .optional()?
            .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
            insert_input(
                conn,
                workflow_id,
                &frame.table_name,
                "model",
                None,
                Some(model.model_version_id),
                frame.artifact_id,
                Some(&frame.table_name),
                None,
                None,
                &frame.content_hash,
                json!({"tableName":frame.table_name,"schema":serde_json::from_str::<serde_json::Value>(&frame.schema_json).unwrap_or_default()}),
                now_ms,
            )?;
        }
    }
    for resource in &mut inputs.resources {
        let resource_id = resource
            .resource_id
            .as_deref()
            .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
        #[derive(QueryableByName)]
        struct Resource {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            artifact_id: i32,
            #[diesel(sql_type = diesel::sql_types::Text)]
            alias: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            schema_hash: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            sha256: String,
        }
        let row = diesel::sql_query(
            "SELECT d.normalized_artifact_id artifact_id,d.alias,d.schema_hash,a.sha256 FROM data_resources d JOIN sessions s ON s.id=d.session_id JOIN artifacts a ON a.id=d.normalized_artifact_id WHERE d.id=?1 AND s.user_id=?2 AND s.deletion_status='live' AND d.status IN ('live','needs_rename')",
        )
        .bind::<diesel::sql_types::Text, _>(resource_id)
        .bind::<diesel::sql_types::Text, _>(user)
        .get_result::<Resource>(conn)
        .optional()?
        .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
        if row.alias != resource.resource_alias
            || row.schema_hash != resource.schema_hash
            || row.sha256 != resource.content_hash
        {
            return Err(ManagedWorkflowRepoError::ConfigurationStale);
        }
        let overlay = overlays
            .iter()
            .find(|overlay| overlay.input_id == resource.input_id);
        let (artifact_id, content_hash, snapshot) = if let Some(overlay) = overlay {
            let prepared_overlay = struct_overlays
                .iter()
                .find(|prepared| prepared.input_id == resource.input_id)
                .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
            let effective_values =
                serde_json::from_str::<serde_json::Value>(&overlay.effective_values_json)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
            let user_overrides =
                serde_json::from_str::<serde_json::Value>(&overlay.user_overrides_json)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
            let selector_overrides =
                serde_json::from_str::<serde_json::Value>(&overlay.selector_overrides_json)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
            let overrides = serde_json::from_str::<serde_json::Value>(&overlay.overrides_json)
                .map_err(|error| RepoError::InvalidState(error.to_string()))?;
            if overlay.source_resource_id.as_deref() != Some(resource_id)
                || overlay.source_artifact_id != Some(row.artifact_id)
                || overlay.source_content_hash.as_deref() != Some(resource.content_hash.as_str())
                || overlay.source_schema_hash.as_deref() != Some(resource.schema_hash.as_str())
                || overlay.schema_ref != resource.schema_ref
                || effective_values != prepared_overlay.effective_values
                || user_overrides != prepared_overlay.user_overrides
                || selector_overrides != prepared_overlay.selector_overrides
                || overrides != prepared_overlay.overrides
            {
                return Err(ManagedWorkflowRepoError::ConfigurationStale);
            }
            (
                overlay.artifact_id,
                overlay.content_hash.clone(),
                json!({
                    "inputId": resource.input_id,
                    "alias": resource.alias,
                    "resourceAlias": resource.resource_alias,
                    "kind": resource.kind,
                    "resourceId": resource.resource_id,
                    "schemaRef": resource.schema_ref,
                    "schemaHash": resource.schema_hash,
                    "contentHash": overlay.content_hash,
                    "source": {
                        "resourceId": resource.resource_id,
                        "resourceAlias": resource.resource_alias,
                        "artifactId": row.artifact_id,
                        "schemaRef": resource.schema_ref,
                        "schemaHash": resource.schema_hash,
                        "contentHash": resource.content_hash,
                    },
                    "effective": {
                        "artifactId": overlay.artifact_id,
                        "contentHash": overlay.content_hash,
                        "values": effective_values,
                        "userOverrides": user_overrides,
                        "selectorOverrides": selector_overrides,
                        "overrides": overrides,
                    }
                }),
            )
        } else {
            (
                row.artifact_id,
                resource.content_hash.clone(),
                serde_json::to_value(&*resource).unwrap(),
            )
        };
        insert_input(
            conn,
            workflow_id,
            &resource.input_id,
            &resource.kind,
            Some(resource_id),
            None,
            artifact_id,
            Some(&resource.alias),
            Some(&resource.schema_ref),
            Some(&resource.schema_hash),
            &content_hash,
            snapshot,
            now_ms,
        )?;
        resource.content_hash = content_hash;
    }
    for overlay in overlays
        .iter()
        .filter(|overlay| overlay.source_resource_id.is_none())
    {
        let prepared_overlay = struct_overlays
            .iter()
            .find(|prepared| prepared.input_id == overlay.input_id)
            .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
        if prepared_overlay.source_resource_id.is_some()
            || inputs
                .resources
                .iter()
                .any(|resource| resource.input_id == overlay.input_id)
            || overlay.source_artifact_id.is_some()
            || overlay.source_content_hash.is_some()
            || overlay.source_schema_hash.is_some()
            || overlay.schema_ref != prepared_overlay.schema_ref
        {
            return Err(ManagedWorkflowRepoError::ConfigurationStale);
        }
        let effective_values =
            serde_json::from_str::<serde_json::Value>(&overlay.effective_values_json)
                .map_err(|error| RepoError::InvalidState(error.to_string()))?;
        let user_overrides =
            serde_json::from_str::<serde_json::Value>(&overlay.user_overrides_json)
                .map_err(|error| RepoError::InvalidState(error.to_string()))?;
        let selector_overrides =
            serde_json::from_str::<serde_json::Value>(&overlay.selector_overrides_json)
                .map_err(|error| RepoError::InvalidState(error.to_string()))?;
        let overrides = serde_json::from_str::<serde_json::Value>(&overlay.overrides_json)
            .map_err(|error| RepoError::InvalidState(error.to_string()))?;
        if effective_values != prepared_overlay.effective_values
            || user_overrides != prepared_overlay.user_overrides
            || selector_overrides != prepared_overlay.selector_overrides
            || overrides != prepared_overlay.overrides
        {
            return Err(ManagedWorkflowRepoError::ConfigurationStale);
        }
        insert_input(
            conn,
            workflow_id,
            &overlay.input_id,
            "struct",
            None,
            None,
            overlay.artifact_id,
            Some(&prepared_overlay.input_alias),
            Some(&overlay.schema_ref),
            Some(&overlay.schema_hash),
            &overlay.content_hash,
            json!({
                "inputId": overlay.input_id,
                "alias": prepared_overlay.input_alias,
                "kind": "struct",
                "resourceId": null,
                "schemaRef": overlay.schema_ref,
                "schemaHash": overlay.schema_hash,
                "contentHash": overlay.content_hash,
                "source": null,
                "effective": {
                    "artifactId": overlay.artifact_id,
                    "contentHash": overlay.content_hash,
                    "values": effective_values,
                    "userOverrides": user_overrides,
                    "selectorOverrides": selector_overrides,
                    "overrides": overrides,
                }
            }),
            now_ms,
        )?;
        inputs.resources.push(FrozenStartResource {
            input_id: overlay.input_id.clone(),
            alias: prepared_overlay.input_alias.clone(),
            resource_alias: prepared_overlay.input_alias.clone(),
            kind: "struct".into(),
            resource_id: None,
            schema_ref: overlay.schema_ref.clone(),
            schema_hash: overlay.schema_hash.clone(),
            content_hash: overlay.content_hash.clone(),
        });
    }
    Ok(())
}

fn validate_active_model(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
    inputs: &FrozenStartInputs,
) -> Result<(), ManagedWorkflowRepoError> {
    let Some(expected) = inputs.model.as_ref() else {
        return Ok(());
    };
    #[derive(QueryableByName)]
    struct ActiveModel {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        version_id: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        content_hash: String,
    }
    let active = diesel::sql_query(
        "SELECT v.id version_id,v.content_hash \
         FROM workspaces w \
         JOIN sessions s ON s.id=w.session_id \
         JOIN model_versions v ON v.id=w.active_version_id AND v.workspace_id=w.id \
         WHERE w.session_id=?1 AND s.user_id=?2 AND s.deletion_status='live' \
           AND v.deleted_at_ms IS NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(session_id)
    .bind::<diesel::sql_types::Text, _>(user)
    .get_result::<ActiveModel>(conn)
    .optional()?
    .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
    if active.version_id != expected.model_version_id
        || active.content_hash != expected.content_hash
    {
        return Err(ManagedWorkflowRepoError::ConfigurationStale);
    }
    let mut frozen = expected.tables.clone();
    frozen.sort();
    frozen.dedup();
    let mut actual = schema::model_dataframes::table
        .filter(schema::model_dataframes::version_id.eq(active.version_id))
        .filter(schema::model_dataframes::table_name.eq_any(&frozen))
        .select(schema::model_dataframes::table_name)
        .load::<String>(conn)?;
    actual.sort();
    if actual != frozen {
        return Err(ManagedWorkflowRepoError::ConfigurationStale);
    }
    Ok(())
}

fn configuration_artifact_error(error: RepoError) -> ManagedWorkflowRepoError {
    match error {
        RepoError::NotFound => ManagedWorkflowRepoError::ConfigurationStale,
        error => ManagedWorkflowRepoError::Repository(error),
    }
}
