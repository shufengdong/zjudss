impl ManagedWorkflowRepo {
    pub async fn find_retry_replay(
        &self,
        user: &UserId,
        script_id: i32,
        client_retry_id: &str,
    ) -> Result<Option<ManagedWorkflowAggregate>, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let client_retry_id = client_retry_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let script = schema::script_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::script_resources::session_id)),
                )
                .filter(schema::script_resources::id.eq(script_id))
                .filter(schema::sessions::user_id.eq(&user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select((
                    schema::script_resources::session_id,
                    schema::script_resources::managed_workflow_id,
                ))
                .first::<(i32, Option<i32>)>(&mut conn)
                .optional()?
                .ok_or(ManagedWorkflowRepoError::Repository(RepoError::NotFound))?;
            let Some(source_workflow_id) = script.1 else {
                return Ok(None);
            };
            let operation = format!("retry:{source_workflow_id}:{client_retry_id}");
            let existing = schema::managed_workflows::table
                .filter(schema::managed_workflows::session_id.eq(script.0))
                .filter(schema::managed_workflows::client_operation_id.eq(Some(operation)))
                .select(schema::managed_workflows::id)
                .first::<i32>(&mut conn)
                .optional()?;
            existing
                .map(|workflow_id| load_owned_on(&mut conn, &user, workflow_id))
                .transpose()
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn preview_retry(
        &self,
        user: &UserId,
        script_id: i32,
        client_retry_id: String,
        engine_snapshot_id: String,
        current_facade_hash: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        token_hash: String,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<RetryPreviewOutcome, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<RetryPreviewOutcome, ManagedWorkflowRepoError, _>(|conn| {
                let material = retry_material(conn, &user, script_id, &client_retry_id, &engine_snapshot_id, &current_facade_hash)?;
                if material.changes.as_array().is_some_and(Vec::is_empty) {
                    let aggregate = insert_retry_workflow(conn, &user, material, correction_budget, &agent_bundle_version, &agent_skill_refs_json, now_ms)?;
                    return Ok(RetryPreviewOutcome { aggregate: Some(aggregate), changes: json!([]) });
                }
                diesel::sql_query("DELETE FROM retry_previews WHERE expires_at_ms<?1 OR (user_id=?2 AND workflow_id=?3)")
                    .bind::<diesel::sql_types::BigInt,_>(now_ms).bind::<diesel::sql_types::Text,_>(&user).bind::<diesel::sql_types::Integer,_>(material.source_workflow_id).execute(conn)?;
                diesel::sql_query("INSERT INTO retry_previews(token_hash,user_id,workflow_id,impact_hash,impact_json,expires_at_ms,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                    .bind::<diesel::sql_types::Text,_>(token_hash).bind::<diesel::sql_types::Text,_>(user).bind::<diesel::sql_types::Integer,_>(material.source_workflow_id).bind::<diesel::sql_types::Text,_>(&material.impact_hash).bind::<diesel::sql_types::Text,_>(&material.impact_json).bind::<diesel::sql_types::BigInt,_>(expires_at_ms).bind::<diesel::sql_types::BigInt,_>(now_ms).execute(conn)?;
                Ok(RetryPreviewOutcome { aggregate: None, changes: material.changes })
            })
        }).await.map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_retry(
        &self,
        user: &UserId,
        token_hash: String,
        overlays: Vec<WorkflowStructOverlayMaterial>,
        engine_snapshot_id: String,
        current_facade_hash: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let inputs = overlays
            .into_iter()
            .map(workflow_overlay_input_from_material)
            .collect();
        self.confirm_retry_with_overlay_artifacts(
            user,
            token_hash,
            inputs,
            Vec::new(),
            |_, _| Ok(()),
            engine_snapshot_id,
            current_facade_hash,
            correction_budget,
            agent_bundle_version,
            agent_skill_refs_json,
            now_ms,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_retry_with_overlay_artifacts<P>(
        &self,
        user: &UserId,
        token_hash: String,
        inputs: Vec<WorkflowOverlayInput>,
        drafts: Vec<ArtifactDraft>,
        publish: P,
        engine_snapshot_id: String,
        current_facade_hash: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError>
    where
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
    {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let scope_id = owned_scope_id(&pool, &user).await?;
        ArtifactRepo::new(pool)
            .commit_batch_with_publish(scope_id, drafts, publish, move |conn, artifacts| {
                let committed = commit_overlay_inputs_on(conn, scope_id, inputs, artifacts)
                    .map_err(|error| match error {
                        RepoError::NotFound => ManagedWorkflowRepoError::RetryImpactChanged,
                        error => ManagedWorkflowRepoError::Repository(error),
                    })?;
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
                #[derive(QueryableByName)] struct Preview { #[diesel(sql_type=diesel::sql_types::Integer)] workflow_id: i32, #[diesel(sql_type=diesel::sql_types::Text)] impact_hash: String, #[diesel(sql_type=diesel::sql_types::Text)] impact_json: String }
                let preview = diesel::sql_query("SELECT workflow_id,impact_hash,impact_json FROM retry_previews WHERE token_hash=?1 AND user_id=?2 AND expires_at_ms>=?3")
                    .bind::<diesel::sql_types::Text,_>(&token_hash).bind::<diesel::sql_types::Text,_>(&user).bind::<diesel::sql_types::BigInt,_>(now_ms).get_result::<Preview>(conn).optional()?.ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
                let stored: serde_json::Value = serde_json::from_str(&preview.impact_json).map_err(|error| RepoError::Storage(error.to_string()))?;
                let script_id = stored.get("scriptId").and_then(serde_json::Value::as_i64).and_then(|id| i32::try_from(id).ok()).ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
                let client_retry_id = stored.get("clientRetryId").and_then(serde_json::Value::as_str).ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
                let mut material = retry_material(conn, &user, script_id, client_retry_id, &engine_snapshot_id, &current_facade_hash)?;
                if material.source_workflow_id != preview.workflow_id || material.impact_hash != preview.impact_hash { return Err(ManagedWorkflowRepoError::RetryImpactChanged); }
                apply_retry_overlays(&mut material, &overlays)?;
                let aggregate = insert_retry_workflow(conn, &user, material, correction_budget, &agent_bundle_version, &agent_skill_refs_json, now_ms)?;
                diesel::delete(schema::retry_previews::table.find(token_hash)).execute(conn)?;
                Ok(aggregate)
                })();
                if result.is_ok() && !adopted.is_empty() {
                    super::artifact_refs::release_refs(conn, scope_id, adopted, now_ms)?;
                }
                result
            })
            .await
    }

    pub async fn retry_locked_contract(
        &self,
        user: &UserId,
        token_hash: &str,
    ) -> Result<String, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::retry_previews::table
                .inner_join(
                    schema::managed_workflows::table
                        .on(schema::managed_workflows::id.eq(schema::retry_previews::workflow_id)),
                )
                .filter(schema::retry_previews::token_hash.eq(token_hash))
                .filter(schema::retry_previews::user_id.eq(user))
                .select(schema::managed_workflows::locked_contract_json)
                .first::<Option<String>>(&mut conn)
                .optional()?
                .flatten()
                .ok_or(ManagedWorkflowRepoError::InvalidRetryToken)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn retry_struct_overlay_requests(
        &self,
        user: &UserId,
        token_hash: &str,
        engine_snapshot_id: &str,
        current_facade_hash: &str,
        now_ms: i64,
    ) -> Result<Vec<StructOverlayRequest>, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let token_hash = token_hash.to_string();
        let engine_snapshot_id = engine_snapshot_id.to_string();
        let current_facade_hash = current_facade_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            #[derive(QueryableByName)]
            struct Preview {
                #[diesel(sql_type = diesel::sql_types::Integer)]
                workflow_id: i32,
                #[diesel(sql_type = diesel::sql_types::Text)]
                impact_hash: String,
                #[diesel(sql_type = diesel::sql_types::Text)]
                impact_json: String,
            }
            let preview = diesel::sql_query("SELECT workflow_id,impact_hash,impact_json FROM retry_previews WHERE token_hash=?1 AND user_id=?2 AND expires_at_ms>=?3")
                .bind::<diesel::sql_types::Text,_>(&token_hash).bind::<diesel::sql_types::Text,_>(&user).bind::<diesel::sql_types::BigInt,_>(now_ms).get_result::<Preview>(&mut conn).optional()?.ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
            let stored: serde_json::Value = serde_json::from_str(&preview.impact_json)
                .map_err(|error| RepoError::Storage(error.to_string()))?;
            let script_id = stored
                .get("scriptId")
                .and_then(serde_json::Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
            let client_retry_id = stored
                .get("clientRetryId")
                .and_then(serde_json::Value::as_str)
                .ok_or(ManagedWorkflowRepoError::InvalidRetryToken)?;
            let material = retry_material(
                &mut conn,
                &user,
                script_id,
                client_retry_id,
                &engine_snapshot_id,
                &current_facade_hash,
            )?;
            if material.source_workflow_id != preview.workflow_id
                || material.impact_hash != preview.impact_hash
            {
                return Err(ManagedWorkflowRepoError::RetryImpactChanged);
            }
            Ok(material.pending_overlays)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RetryFrozenInput {
    input_name: String,
    input_kind: String,
    resource_id: Option<String>,
    model_version_id: Option<i32>,
    artifact_id: i32,
    input_alias: String,
    resource_alias: Option<String>,
    schema_ref: Option<String>,
    schema_hash: Option<String>,
    content_hash: String,
    snapshot: serde_json::Value,
}

struct RetryMaterial {
    source_workflow_id: i32,
    session_id: i32,
    configuration_id: Option<i32>,
    configuration_revision: Option<i64>,
    client_retry_id: String,
    intent_json: String,
    locked_contract_json: String,
    engine_snapshot_id: String,
    facade_contract_hash: String,
    result_contract_json: String,
    inputs: Vec<RetryFrozenInput>,
    pending_overlays: Vec<StructOverlayRequest>,
    input_snapshot_json: String,
    changes: serde_json::Value,
    impact_json: String,
    impact_hash: String,
}

fn retry_material(
    conn: &mut SqliteConnection,
    user: &str,
    script_id: i32,
    client_retry_id: &str,
    engine_snapshot_id: &str,
    current_facade_hash: &str,
) -> Result<RetryMaterial, ManagedWorkflowRepoError> {
    let script = schema::script_resources::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::script_resources::session_id)),
        )
        .filter(schema::script_resources::id.eq(script_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::script_resources::all_columns)
        .first::<crate::db::models::ScriptResource>(conn)
        .optional()?
        .ok_or(ManagedWorkflowRepoError::Repository(RepoError::NotFound))?;
    let source_workflow_id = script
        .managed_workflow_id
        .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
    let source = load_owned_on(conn, user, source_workflow_id)?;
    if source.workflow.status != "failed"
        || source.workflow.final_script_resource_id != Some(script_id)
    {
        return Err(ManagedWorkflowRepoError::RetryInputUnavailable);
    }
    if source.workflow.facade_contract_hash.as_deref() != Some(current_facade_hash) {
        return Err(ManagedWorkflowRepoError::StudyContractChanged);
    }
    let locked_contract_json = source
        .workflow
        .locked_contract_json
        .clone()
        .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
    let locked: serde_json::Value = serde_json::from_str(&locked_contract_json)
        .map_err(|error| RepoError::Storage(error.to_string()))?;
    let result_contract_json = locked
        .get("results")
        .cloned()
        .unwrap_or_else(|| json!([]))
        .to_string();
    #[derive(QueryableByName)]
    struct Active {
        #[diesel(sql_type=diesel::sql_types::Integer)]
        version_id: i32,
        #[diesel(sql_type=diesel::sql_types::Text)]
        content_hash: String,
    }
    let active = diesel::sql_query("SELECT v.id version_id,v.content_hash FROM workspaces w JOIN model_versions v ON v.id=w.active_version_id WHERE w.session_id=?1 AND v.deleted_at_ms IS NULL")
        .bind::<diesel::sql_types::Integer,_>(source.workflow.session_id).get_result::<Active>(conn).optional()?;
    let mut inputs = Vec::new();
    let mut pending_overlays = Vec::new();
    let mut changes = Vec::new();
    for frozen in &source.inputs {
        if frozen.input_kind == "model" {
            let active = active
                .as_ref()
                .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
            #[derive(QueryableByName)]
            struct Frame {
                #[diesel(sql_type=diesel::sql_types::Integer)]
                artifact_id: i32,
                #[diesel(sql_type=diesel::sql_types::Text)]
                content_hash: String,
                #[diesel(sql_type=diesel::sql_types::Text)]
                schema_json: String,
            }
            let alias = frozen.input_alias.as_deref().unwrap_or(&frozen.input_name);
            let frame = diesel::sql_query("SELECT artifact_id,content_hash,schema_json FROM model_dataframes WHERE version_id=?1 AND table_name=?2")
                .bind::<diesel::sql_types::Integer,_>(active.version_id).bind::<diesel::sql_types::Text,_>(alias).get_result::<Frame>(conn).optional()?.ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
            if frozen.content_hash.as_deref() != Some(&frame.content_hash)
                || frozen.model_version_id != Some(active.version_id)
            {
                changes.push(json!({"input":frozen.input_name,"kind":"model","before":{"modelVersionId":frozen.model_version_id,"contentHash":frozen.content_hash},"after":{"modelVersionId":active.version_id,"contentHash":frame.content_hash}}));
            }
            inputs.push(RetryFrozenInput { input_name: frozen.input_name.clone(), input_kind: "model".into(), resource_id: None, model_version_id: Some(active.version_id), artifact_id: frame.artifact_id, input_alias: alias.into(), resource_alias: None, schema_ref: None, schema_hash: None, content_hash: frame.content_hash.clone(), snapshot: json!({"tableName":alias,"schema":serde_json::from_str::<serde_json::Value>(&frame.schema_json).unwrap_or_default(),"modelContentHash":active.content_hash}) });
        } else {
            let input_alias = frozen
                .input_alias
                .as_deref()
                .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
            if frozen.input_kind == "struct" {
                let resolved =
                    resolve_frozen_struct_input_on(conn, source.workflow.session_id, frozen)
                        .map_err(|error| match error {
                            RepoError::InvalidState(_) => {
                                ManagedWorkflowRepoError::ConfigurationStale
                            }
                            error => ManagedWorkflowRepoError::Repository(error),
                        })?;
                if resolved.source_changed {
                    let resource = resolved
                        .resource
                        .as_ref()
                        .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
                    changes.push(json!({"input":frozen.input_name,"kind":frozen.input_kind,"before":{"resourceId":resolved.frozen_resource_id,"contentHash":resolved.frozen_content_hash,"schemaHash":resolved.frozen_schema_hash},"after":{"resourceId":resource.id,"contentHash":resource.content_hash,"schemaHash":resource.schema_hash}}));
                }
                if let Some(request) = resolved.request.clone() {
                    pending_overlays.push(request);
                }
                let Some(resource) = resolved.resource else {
                    inputs.push(RetryFrozenInput {
                        input_name: frozen.input_name.clone(),
                        input_kind: frozen.input_kind.clone(),
                        resource_id: None,
                        model_version_id: None,
                        artifact_id: resolved.effective_artifact_id,
                        input_alias: input_alias.into(),
                        resource_alias: Some(input_alias.into()),
                        schema_ref: frozen.schema_ref.clone(),
                        schema_hash: frozen.schema_hash.clone(),
                        content_hash: resolved.effective_content_hash,
                        snapshot: resolved.snapshot,
                    });
                    continue;
                };
                if !resolved.source_changed && resolved.snapshot.get("effective").is_some() {
                    inputs.push(RetryFrozenInput {
                        input_name: frozen.input_name.clone(),
                        input_kind: frozen.input_kind.clone(),
                        resource_id: Some(resource.id.clone()),
                        model_version_id: None,
                        artifact_id: resolved.effective_artifact_id,
                        input_alias: input_alias.into(),
                        resource_alias: Some(resource.alias.clone()),
                        schema_ref: Some(resource.schema_ref.clone()),
                        schema_hash: Some(resource.schema_hash.clone()),
                        content_hash: resolved.effective_content_hash,
                        snapshot: resolved.snapshot,
                    });
                } else {
                    inputs.push(RetryFrozenInput { input_name: frozen.input_name.clone(), input_kind: frozen.input_kind.clone(), resource_id: Some(resource.id.clone()), model_version_id: None, artifact_id: resource.artifact_id, input_alias: input_alias.into(), resource_alias: Some(resource.alias.clone()), schema_ref: Some(resource.schema_ref.clone()), schema_hash: Some(resource.schema_hash.clone()), content_hash: resource.content_hash.clone(), snapshot: json!({"resourceId":resource.id,"alias":input_alias,"resourceAlias":resource.alias,"schemaRef":resource.schema_ref,"schemaHash":resource.schema_hash,"contentHash":resource.content_hash}) });
                }
                continue;
            }
            let frozen_snapshot: serde_json::Value = serde_json::from_str(&frozen.snapshot_json)
                .map_err(|error| RepoError::Storage(error.to_string()))?;
            let frozen_source = frozen_snapshot.get("source").unwrap_or(&frozen_snapshot);
            let frozen_resource_id = frozen_source
                .get("resourceId")
                .and_then(serde_json::Value::as_str)
                .or(frozen.resource_id.as_deref());
            if frozen_resource_id.is_none() {
                if !frozen_snapshot
                    .get("source")
                    .is_some_and(serde_json::Value::is_null)
                    && frozen.resource_id.is_some()
                {
                    return Err(ManagedWorkflowRepoError::ConfigurationStale);
                }
                inputs.push(RetryFrozenInput {
                    input_name: frozen.input_name.clone(),
                    input_kind: frozen.input_kind.clone(),
                    resource_id: None,
                    model_version_id: None,
                    artifact_id: frozen
                        .artifact_id
                        .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?,
                    input_alias: input_alias.into(),
                    resource_alias: Some(input_alias.into()),
                    schema_ref: frozen.schema_ref.clone(),
                    schema_hash: frozen.schema_hash.clone(),
                    content_hash: frozen
                        .content_hash
                        .clone()
                        .ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?,
                    snapshot: frozen_snapshot,
                });
                continue;
            }
            let resource_alias = frozen_source
                .get("resourceAlias")
                .or_else(|| frozen_snapshot.get("resourceAlias"))
                .or_else(|| frozen_snapshot.get("resource_alias"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(input_alias);
            let expected_schema_ref = frozen
                .schema_ref
                .as_deref()
                .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
            #[derive(QueryableByName)]
            struct Resource {
                #[diesel(sql_type=diesel::sql_types::Text)]
                id: String,
                #[diesel(sql_type=diesel::sql_types::Integer)]
                artifact_id: i32,
                #[diesel(sql_type=diesel::sql_types::Text)]
                alias: String,
                #[diesel(sql_type=diesel::sql_types::Text)]
                schema_ref: String,
                #[diesel(sql_type=diesel::sql_types::Text)]
                schema_hash: String,
                #[diesel(sql_type=diesel::sql_types::Text)]
                content_hash: String,
            }
            let current = diesel::sql_query("SELECT d.id,d.normalized_artifact_id artifact_id,d.alias,d.declared_schema_ref schema_ref,d.schema_hash,a.sha256 content_hash FROM data_resources d JOIN artifacts a ON a.id=d.normalized_artifact_id WHERE d.session_id=?1 AND lower(d.alias)=lower(?2) AND d.status='live'")
                .bind::<diesel::sql_types::Integer,_>(source.workflow.session_id).bind::<diesel::sql_types::Text,_>(resource_alias).get_result::<Resource>(conn).optional()?.ok_or(ManagedWorkflowRepoError::RetryInputUnavailable)?;
            if current.schema_ref != expected_schema_ref {
                return Err(ManagedWorkflowRepoError::RetryInputUnavailable);
            }
            let resource = current;
            let frozen_source_hash = frozen_source
                .get("contentHash")
                .and_then(serde_json::Value::as_str)
                .or(frozen.content_hash.as_deref());
            let frozen_schema_hash = frozen_source
                .get("schemaHash")
                .and_then(serde_json::Value::as_str)
                .or(frozen.schema_hash.as_deref());
            let changed = frozen_resource_id != Some(&resource.id)
                || frozen_source_hash != Some(&resource.content_hash)
                || frozen_schema_hash != Some(&resource.schema_hash);
            if changed {
                changes.push(json!({"input":frozen.input_name,"kind":frozen.input_kind,"before":{"resourceId":frozen_resource_id,"contentHash":frozen_source_hash,"schemaHash":frozen_schema_hash},"after":{"resourceId":resource.id,"contentHash":resource.content_hash,"schemaHash":resource.schema_hash}}));
            }
            inputs.push(RetryFrozenInput { input_name: frozen.input_name.clone(), input_kind: frozen.input_kind.clone(), resource_id: Some(resource.id.clone()), model_version_id: None, artifact_id: resource.artifact_id, input_alias: input_alias.into(), resource_alias: Some(resource.alias.clone()), schema_ref: Some(resource.schema_ref.clone()), schema_hash: Some(resource.schema_hash.clone()), content_hash: resource.content_hash.clone(), snapshot: json!({"resourceId":resource.id,"alias":input_alias,"resourceAlias":resource.alias,"schemaRef":resource.schema_ref,"schemaHash":resource.schema_hash,"contentHash":resource.content_hash}) });
        }
    }
    let input_snapshot_json = retry_input_snapshot_json(&inputs);
    let changes = serde_json::Value::Array(changes);
    let impact = json!({"sourceWorkflowId":source_workflow_id,"scriptId":script_id,"clientRetryId":client_retry_id,"engineSnapshotId":engine_snapshot_id,"facadeContractHash":current_facade_hash,"inputs":inputs,"pendingOverlays":pending_overlays,"changes":changes});
    let impact_json = crate::application::canonical_json(&impact);
    let impact_hash = hex::encode(sha2::Sha256::digest(impact_json.as_bytes()));
    Ok(RetryMaterial {
        source_workflow_id,
        session_id: source.workflow.session_id,
        configuration_id: source.workflow.configuration_id,
        configuration_revision: source.workflow.configuration_revision,
        client_retry_id: client_retry_id.into(),
        intent_json: source.workflow.intent_json,
        locked_contract_json,
        engine_snapshot_id: engine_snapshot_id.into(),
        facade_contract_hash: current_facade_hash.into(),
        result_contract_json,
        inputs,
        pending_overlays,
        input_snapshot_json,
        changes,
        impact_json,
        impact_hash,
    })
}

fn retry_input_snapshot_json(inputs: &[RetryFrozenInput]) -> String {
    let model_inputs = inputs
        .iter()
        .filter(|input| input.input_kind == "model")
        .collect::<Vec<_>>();
    let model_content_hash = model_inputs.first().and_then(|input| {
        input
            .snapshot
            .get("modelContentHash")
            .and_then(serde_json::Value::as_str)
    });
    json!({
        "model": model_inputs.first().map(|first| json!({"modelVersionId":first.model_version_id,"contentHash":model_content_hash,"tables":model_inputs.iter().map(|input|input.input_alias.clone()).collect::<Vec<_>>()})),
        "resources": inputs.iter().filter(|input| input.input_kind != "model").map(|input|json!({"inputId":input.input_name,"alias":input.input_alias,"resourceAlias":input.resource_alias,"kind":input.input_kind,"resourceId":input.resource_id,"schemaRef":input.schema_ref,"schemaHash":input.schema_hash,"contentHash":input.content_hash})).collect::<Vec<_>>()
    }).to_string()
}

fn apply_retry_overlays(
    material: &mut RetryMaterial,
    overlays: &[WorkflowStructOverlayMaterial],
) -> Result<(), ManagedWorkflowRepoError> {
    if material.pending_overlays.len() != overlays.len() {
        return Err(ManagedWorkflowRepoError::RetryImpactChanged);
    }
    for request in &material.pending_overlays {
        let overlay = overlays
            .iter()
            .find(|overlay| overlay.input_id == request.input_id)
            .ok_or(ManagedWorkflowRepoError::RetryImpactChanged)?;
        if overlay.source_resource_id.as_deref() != Some(request.source_resource_id.as_str())
            || overlay.source_artifact_id != Some(request.source_artifact_id)
            || overlay.source_content_hash.as_deref() != Some(request.source_content_hash.as_str())
            || overlay.source_schema_hash.as_deref() != Some(request.source_schema_hash.as_str())
            || overlay.schema_ref != request.schema_ref
            || serde_json::from_str::<serde_json::Value>(&overlay.overrides_json)
                .ok()
                .as_ref()
                != Some(&request.overrides)
        {
            return Err(ManagedWorkflowRepoError::RetryImpactChanged);
        }
        let input = material
            .inputs
            .iter_mut()
            .find(|input| input.input_name == request.input_id)
            .ok_or(ManagedWorkflowRepoError::RetryImpactChanged)?;
        let source = json!({
            "resourceId": overlay.source_resource_id,
            "resourceAlias": input.resource_alias,
            "artifactId": overlay.source_artifact_id,
            "schemaRef": overlay.schema_ref,
            "schemaHash": overlay.source_schema_hash,
            "contentHash": overlay.source_content_hash,
        });
        input.artifact_id = overlay.artifact_id;
        input.content_hash = overlay.content_hash.clone();
        input.snapshot = json!({
            "resourceId": overlay.source_resource_id,
            "alias": input.input_alias,
            "resourceAlias": input.resource_alias,
            "schemaRef": overlay.schema_ref,
            "schemaHash": overlay.source_schema_hash,
            "contentHash": overlay.content_hash,
            "source": source,
            "effective": {
                "artifactId": overlay.artifact_id,
                "contentHash": overlay.content_hash,
                "values": serde_json::from_str::<serde_json::Value>(&overlay.effective_values_json)
                    .map_err(|error| RepoError::Storage(error.to_string()))?,
                "userOverrides": serde_json::from_str::<serde_json::Value>(&overlay.user_overrides_json)
                    .map_err(|error| RepoError::Storage(error.to_string()))?,
                "selectorOverrides": serde_json::from_str::<serde_json::Value>(&overlay.selector_overrides_json)
                    .map_err(|error| RepoError::Storage(error.to_string()))?,
                "overrides": request.overrides,
            },
        });
    }
    material.input_snapshot_json = retry_input_snapshot_json(&material.inputs);
    material.pending_overlays.clear();
    Ok(())
}

fn insert_retry_workflow(
    conn: &mut SqliteConnection,
    user: &str,
    material: RetryMaterial,
    correction_budget: i32,
    agent_bundle_version: &str,
    agent_skill_refs_json: &str,
    now_ms: i64,
) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
    let operation = format!(
        "retry:{}:{}",
        material.source_workflow_id, material.client_retry_id
    );
    if let Some(existing) = schema::managed_workflows::table
        .filter(schema::managed_workflows::session_id.eq(material.session_id))
        .filter(schema::managed_workflows::client_operation_id.eq(&operation))
        .select(schema::managed_workflows::id)
        .first::<i32>(conn)
        .optional()?
    {
        return load_owned_on(conn, user, existing);
    }
    if let Some(active) = schema::managed_workflows::table
        .filter(schema::managed_workflows::session_id.eq(material.session_id))
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
        return Err(ManagedWorkflowRepoError::WorkflowInProgress(active));
    }
    diesel::insert_into(schema::managed_workflows::table)
        .values((
            schema::managed_workflows::session_id.eq(material.session_id),
            schema::managed_workflows::configuration_id.eq(material.configuration_id),
            schema::managed_workflows::configuration_revision.eq(material.configuration_revision),
            schema::managed_workflows::source_workflow_id.eq(Some(material.source_workflow_id)),
            schema::managed_workflows::client_operation_id.eq(Some(operation)),
            schema::managed_workflows::status.eq("queued"),
            schema::managed_workflows::intent_json.eq(material.intent_json),
            schema::managed_workflows::locked_contract_json
                .eq(Some(material.locked_contract_json.clone())),
            schema::managed_workflows::input_diff_json.eq(Some(material.changes.to_string())),
            schema::managed_workflows::engine_snapshot_id
                .eq(Some(material.engine_snapshot_id.clone())),
            schema::managed_workflows::facade_contract_hash
                .eq(Some(material.facade_contract_hash.clone())),
            schema::managed_workflows::correction_budget.eq(correction_budget),
            schema::managed_workflows::corrections_used.eq(0),
            schema::managed_workflows::created_at_ms.eq(now_ms),
            schema::managed_workflows::updated_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    let workflow_id = last_id(conn)?;
    for input in &material.inputs {
        insert_input(
            conn,
            workflow_id,
            &input.input_name,
            &input.input_kind,
            input.resource_id.as_deref(),
            input.model_version_id,
            input.artifact_id,
            Some(&input.input_alias),
            input.schema_ref.as_deref(),
            input.schema_hash.as_deref(),
            &input.content_hash,
            input.snapshot.clone(),
            now_ms,
        )?;
    }
    let engine_contract_version = schema::engine_resource_snapshots::table
        .find(&material.engine_snapshot_id)
        .select(schema::engine_resource_snapshots::engine_contract_version)
        .first::<String>(conn)?;
    let model_version_id = material
        .inputs
        .iter()
        .find(|input| input.input_kind == "model")
        .and_then(|input| input.model_version_id);
    let model_content_hash = material
        .inputs
        .iter()
        .find(|input| input.input_kind == "model")
        .and_then(|input| input.snapshot.get("modelContentHash"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    diesel::insert_into(schema::simulation_runs::table)
        .values((
            schema::simulation_runs::session_id.eq(material.session_id),
            schema::simulation_runs::model_version_id.eq(model_version_id),
            schema::simulation_runs::model_content_hash.eq(model_content_hash),
            schema::simulation_runs::script_resource_id.eq(Option::<i32>::None),
            schema::simulation_runs::script_artifact_id.eq(Option::<i32>::None),
            schema::simulation_runs::script_content_hash.eq(Option::<String>::None),
            schema::simulation_runs::client_run_id.eq(format!("managed:{workflow_id}")),
            schema::simulation_runs::status.eq("queued"),
            schema::simulation_runs::stage.eq("generating"),
            schema::simulation_runs::engine_contract_version.eq(engine_contract_version),
            schema::simulation_runs::agent_bundle_version.eq(Some(agent_bundle_version)),
            schema::simulation_runs::agent_skill_refs_json.eq(agent_skill_refs_json),
            schema::simulation_runs::managed_workflow_id.eq(Some(workflow_id)),
            schema::simulation_runs::run_kind.eq("managed"),
            schema::simulation_runs::input_snapshot_json.eq(material.input_snapshot_json),
            schema::simulation_runs::result_contract_json.eq(Some(material.result_contract_json)),
            schema::simulation_runs::created_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    let run_id = last_id(conn)?;
    let payload = json!({"workflowId":workflow_id.to_string(),"state":"queued","executionIndex":0,"executionMax":correction_budget+1,"repairIndex":0,"repairMax":correction_budget,"elapsedMs":0,"attempts":[],"outputDatasetIds":[]}).to_string();
    diesel::insert_into(schema::messages::table)
        .values((
            schema::messages::session_id.eq(material.session_id),
            schema::messages::run_id.eq(Option::<i32>::None),
            schema::messages::operation_id.eq(Some(workflow_id.to_string())),
            schema::messages::revision.eq(1),
            schema::messages::role.eq("tool"),
            schema::messages::kind.eq("workflow-progress"),
            schema::messages::content.eq("研究工作流：queued"),
            schema::messages::payload_json.eq(Some(payload)),
            schema::messages::created_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    let progress_message_id = last_id(conn)?;
    diesel::update(schema::managed_workflows::table.find(workflow_id))
        .set((
            schema::managed_workflows::progress_message_id.eq(Some(progress_message_id)),
            schema::managed_workflows::final_run_id.eq(Some(run_id)),
        ))
        .execute(conn)?;
    load_owned_on(conn, user, workflow_id)
}
