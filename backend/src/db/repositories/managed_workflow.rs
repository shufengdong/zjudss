use diesel::prelude::*;
use serde_json::json;
use sha2::Digest;

use crate::application::PreparedStudyConfirmation;
use crate::auth::UserId;
use crate::db::models::{
    ManagedWorkflowRecord, SimulationRun, WorkflowAttemptRecord, WorkflowInputSnapshotRecord,
};
use crate::db::{DbPool, schema};

use super::{
    ArtifactDraft, ArtifactRepo, CommittedOverlayInput, NewRunDataset, RepoError, RepoResult,
    StructOverlayRequest, WorkflowOverlayInput, commit_overlay_inputs_on,
    resolve_frozen_struct_input_on,
};

#[derive(Debug, Clone)]
pub struct ManagedWorkflowAggregate {
    pub workflow: ManagedWorkflowRecord,
    pub run: SimulationRun,
    pub attempts: Vec<WorkflowAttemptRecord>,
    pub inputs: Vec<WorkflowInputSnapshotRecord>,
    pub output_dataset_ids: Vec<i32>,
    pub progress_message_revision: i32,
}

#[derive(Debug)]
pub struct ConfirmStartOutcome {
    pub aggregate: ManagedWorkflowAggregate,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub struct RetryPreviewOutcome {
    pub aggregate: Option<ManagedWorkflowAggregate>,
    pub changes: serde_json::Value,
}

#[derive(Debug)]
pub enum ManagedWorkflowRepoError {
    Repository(RepoError),
    WorkflowInProgress(i32),
    IdempotencyConflict,
    ConfigurationStale,
    TerminalConflict,
    StudyContractChanged,
    RetryInputUnavailable,
    InvalidRetryToken,
    RetryImpactChanged,
}

impl std::fmt::Display for ManagedWorkflowRepoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::WorkflowInProgress(id) => write!(formatter, "workflow {id} is already running"),
            Self::IdempotencyConflict => write!(formatter, "idempotency conflict"),
            Self::ConfigurationStale => write!(formatter, "configuration stale"),
            Self::TerminalConflict => write!(formatter, "workflow terminal conflict"),
            Self::StudyContractChanged => write!(formatter, "study contract changed"),
            Self::RetryInputUnavailable => write!(formatter, "retry input unavailable"),
            Self::InvalidRetryToken => write!(formatter, "invalid retry token"),
            Self::RetryImpactChanged => write!(formatter, "retry impact changed"),
        }
    }
}

impl std::error::Error for ManagedWorkflowRepoError {}

impl From<RepoError> for ManagedWorkflowRepoError {
    fn from(value: RepoError) -> Self {
        Self::Repository(value)
    }
}

impl From<diesel::result::Error> for ManagedWorkflowRepoError {
    fn from(value: diesel::result::Error) -> Self {
        Self::Repository(RepoError::Query(value))
    }
}

#[derive(Clone)]
pub struct ManagedWorkflowRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct NewWorkflowCandidate {
    pub phase: String,
    pub parent_attempt_id: Option<i32>,
    pub artifact_id: i32,
    pub content_hash: String,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub agent_model: Option<String>,
    pub engine_snapshot_id: String,
    pub now_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowAttemptUpdate {
    pub status: String,
    pub workflow_status: String,
    pub exit_code: Option<i32>,
    pub stderr_audit_artifact_id: Option<i32>,
    pub sanitized_stderr: Option<String>,
    pub stderr_full_hash: Option<String>,
    pub stderr_retained_hash: Option<String>,
    pub stderr_sent_hash: Option<String>,
    pub stderr_total_bytes: Option<i64>,
    pub stderr_sent_bytes: Option<i64>,
    pub stderr_truncated: bool,
    pub diagnostic_json: Option<String>,
    pub started: bool,
    pub finished: bool,
    pub now_ms: i64,
}

#[derive(Debug, Clone)]
pub struct PendingRunDataset {
    pub result_name: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowStructOverlayMaterial {
    pub input_id: String,
    pub source_resource_id: Option<String>,
    pub source_artifact_id: Option<i32>,
    pub source_content_hash: Option<String>,
    pub source_schema_hash: Option<String>,
    pub artifact_id: i32,
    pub content_hash: String,
    pub schema_ref: String,
    pub schema_hash: String,
    pub effective_values_json: String,
    pub user_overrides_json: String,
    pub selector_overrides_json: String,
    pub overrides_json: String,
    pub owns_artifact_reference: bool,
}

impl From<CommittedOverlayInput> for WorkflowStructOverlayMaterial {
    fn from(value: CommittedOverlayInput) -> Self {
        Self {
            input_id: value.input_id,
            source_resource_id: value.source_resource_id,
            source_artifact_id: value.source_artifact_id,
            source_content_hash: value.source_content_hash,
            source_schema_hash: value.source_schema_hash,
            artifact_id: value.artifact_id,
            content_hash: value.content_hash,
            schema_ref: value.schema_ref,
            schema_hash: value.schema_hash,
            effective_values_json: value.effective_values_json,
            user_overrides_json: value.user_overrides_json,
            selector_overrides_json: value.selector_overrides_json,
            overrides_json: value.overrides_json,
            owns_artifact_reference: value.adopts_batch_link,
        }
    }
}

fn workflow_overlay_input_from_material(
    overlay: WorkflowStructOverlayMaterial,
) -> WorkflowOverlayInput {
    WorkflowOverlayInput {
        input_id: overlay.input_id,
        source_resource_id: overlay.source_resource_id,
        source_artifact_id: overlay.source_artifact_id,
        source_content_hash: overlay.source_content_hash,
        source_schema_hash: overlay.source_schema_hash,
        schema_ref: overlay.schema_ref,
        schema_hash: overlay.schema_hash,
        effective_values_json: overlay.effective_values_json,
        user_overrides_json: overlay.user_overrides_json,
        selector_overrides_json: overlay.selector_overrides_json,
        overrides_json: overlay.overrides_json,
        artifact: super::OverlayArtifactSource::Existing {
            artifact_id: overlay.artifact_id,
            content_hash: overlay.content_hash,
        },
    }
}

impl ManagedWorkflowRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

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
                .map(|workflow_id| load_owned(&mut conn, &user, workflow_id))
                .transpose()
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

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
            load_owned(&mut conn, &user, existing.id).map(Some)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_and_start(
        &self,
        user: &UserId,
        prepared: PreparedStudyConfirmation,
        overlays: Vec<WorkflowStructOverlayMaterial>,
        client_operation_id: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        now_ms: i64,
    ) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError> {
        let inputs = overlays
            .into_iter()
            .map(workflow_overlay_input_from_material)
            .collect();
        self.confirm_and_start_with_overlay_artifacts(
            user,
            prepared,
            inputs,
            Vec::new(),
            |_, _| Ok(()),
            client_operation_id,
            correction_budget,
            agent_bundle_version,
            agent_skill_refs_json,
            now_ms,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_and_start_with_overlay_artifacts<P>(
        &self,
        user: &UserId,
        mut prepared: PreparedStudyConfirmation,
        inputs: Vec<WorkflowOverlayInput>,
        drafts: Vec<ArtifactDraft>,
        publish: P,
        client_operation_id: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        now_ms: i64,
    ) -> Result<ConfirmStartOutcome, ManagedWorkflowRepoError>
    where
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
    {
        let pool = self.pool.clone();
        let user = user.0.clone();
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
                            .filter(schema::study_configurations::id.eq(prepared.configuration_id))
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
                                .eq(Some(client_operation_id.clone())),
                        )
                        .first::<ManagedWorkflowRecord>(conn)
                        .optional()?
                    {
                        if existing.configuration_id != Some(prepared.configuration_id)
                            || existing.configuration_revision != Some(prepared.expected_revision)
                        {
                            return Err(ManagedWorkflowRepoError::IdempotencyConflict);
                        }
                        return load_owned(conn, &user, existing.id).map(|aggregate| {
                            ConfirmStartOutcome {
                                aggregate,
                                created: false,
                            }
                        });
                    }

                    if configuration.revision != prepared.expected_revision
                        || configuration.status != "ready_to_confirm"
                        || configuration.engine_snapshot_id != prepared.expected_snapshot_id
                        || configuration.facade_hash != prepared.locked.facade_contract_hash
                    {
                        return Err(ManagedWorkflowRepoError::ConfigurationStale);
                    }
                    if prepared.struct_overlays.len() != overlays.len() {
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

                    validate_active_model(conn, &user, configuration.session_id, &prepared)?;

                    let locked_json = serde_json::to_string(&prepared.locked)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    diesel::insert_into(schema::managed_workflows::table)
                        .values((
                            schema::managed_workflows::session_id.eq(configuration.session_id),
                            schema::managed_workflows::configuration_id.eq(Some(configuration.id)),
                            schema::managed_workflows::configuration_revision
                                .eq(Some(configuration.revision)),
                            schema::managed_workflows::source_workflow_id.eq(Option::<i32>::None),
                            schema::managed_workflows::client_operation_id
                                .eq(Some(client_operation_id)),
                            schema::managed_workflows::status.eq("queued"),
                            schema::managed_workflows::intent_json.eq(&configuration.intent_json),
                            schema::managed_workflows::locked_contract_json.eq(Some(&locked_json)),
                            schema::managed_workflows::input_diff_json.eq(Option::<String>::None),
                            schema::managed_workflows::engine_snapshot_id
                                .eq(Some(&prepared.expected_snapshot_id)),
                            schema::managed_workflows::facade_contract_hash
                                .eq(Some(&prepared.locked.facade_contract_hash)),
                            schema::managed_workflows::correction_budget.eq(correction_budget),
                            schema::managed_workflows::corrections_used.eq(0),
                            schema::managed_workflows::created_at_ms.eq(now_ms),
                            schema::managed_workflows::updated_at_ms.eq(now_ms),
                        ))
                        .execute(conn)?;
                    let workflow_id = last_id(conn)?;

                    freeze_inputs(conn, &user, workflow_id, &mut prepared, &overlays, now_ms)?;
                    let inputs_json = serde_json::to_string(&prepared.inputs)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    let engine_contract_version = schema::engine_resource_snapshots::table
                        .find(&prepared.expected_snapshot_id)
                        .select(schema::engine_resource_snapshots::engine_contract_version)
                        .first::<String>(conn)?;
                    diesel::insert_into(schema::simulation_runs::table)
                        .values((
                            schema::simulation_runs::session_id.eq(configuration.session_id),
                            schema::simulation_runs::model_version_id.eq(prepared
                                .inputs
                                .model
                                .as_ref()
                                .map(|model| model.model_version_id)),
                            schema::simulation_runs::model_content_hash.eq(prepared
                                .inputs
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
                                .eq(Some(agent_bundle_version)),
                            schema::simulation_runs::agent_skill_refs_json
                                .eq(agent_skill_refs_json),
                            schema::simulation_runs::managed_workflow_id.eq(Some(workflow_id)),
                            schema::simulation_runs::run_kind.eq("managed"),
                            schema::simulation_runs::input_snapshot_json.eq(inputs_json),
                            schema::simulation_runs::result_contract_json.eq(Some(
                                serde_json::to_string(&prepared.locked.results).unwrap(),
                            )),
                            schema::simulation_runs::created_at_ms.eq(now_ms),
                        ))
                        .execute(conn)?;
                    let run_id = last_id(conn)?;
                    let payload = json!({
                        "workflowId": workflow_id.to_string(),
                        "state": "queued",
                        "executionIndex": 0,
                        "executionMax": correction_budget + 1,
                        "repairIndex": 0,
                        "repairMax": correction_budget,
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
                            schema::messages::created_at_ms.eq(now_ms),
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
                        schema::study_configurations::confirmed_at_ms.eq(Some(now_ms)),
                        schema::study_configurations::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                    if updated != 1 {
                        return Err(ManagedWorkflowRepoError::ConfigurationStale);
                    }
                    load_owned(conn, &user, workflow_id).map(|aggregate| ConfirmStartOutcome {
                        aggregate,
                        created: true,
                    })
                })();
                if result.is_ok() && !adopted.is_empty() {
                    super::artifact_refs::release_refs(conn, scope_id, adopted, now_ms)?;
                }
                result
            })
            .await
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

    pub async fn find(
        &self,
        user: &UserId,
        workflow_id: i32,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            load_owned(&mut conn, &user, workflow_id)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn cancel(
        &self,
        user: &UserId,
        workflow_id: i32,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned(conn, &user, workflow_id)?;
                if matches!(
                    current.workflow.status.as_str(),
                    "succeeded" | "failed" | "service_restarted"
                ) {
                    return Ok(current);
                }
                if current.workflow.status != "cancelled_by_user" {
                    terminalize(
                        conn,
                        &user,
                        &current,
                        None,
                        "cancelled_by_user",
                        "cancelled",
                        Some(
                            json!({"code":"cancelled_by_user","message":"用户已取消研究任务"})
                                .to_string(),
                        ),
                        now_ms,
                        &[],
                    )?;
                }
                load_owned(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }

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
                let current = load_owned(conn, &user, workflow_id)?;
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
                load_owned(conn, &user, workflow_id)
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
                let current = load_owned(conn, &user, workflow_id)?;
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
                load_owned(conn, &user, workflow_id)
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
                let current = load_owned(conn, &user, workflow_id)?;
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
                load_owned(conn, &user, workflow_id)
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
                let current = load_owned(conn, &user, workflow_id)?;
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
                load_owned(conn, &user, workflow_id)
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
                let current = load_owned(conn, &owner, workflow_id)
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

pub(crate) fn recover_interrupted_workflows(
    conn: &mut SqliteConnection,
    now_ms: i64,
) -> Result<(), diesel::result::Error> {
    #[derive(QueryableByName)]
    struct Interrupted {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        id: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        user_id: String,
    }
    let interrupted = diesel::sql_query(
        "SELECT w.id,s.user_id FROM managed_workflows w JOIN sessions s ON s.id=w.session_id \
         WHERE w.status IN ('queued','generating','executing','repairing') \
           AND s.deletion_status='live' \
         ORDER BY w.id",
    )
    .load::<Interrupted>(conn)?;
    for item in interrupted {
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let current =
                load_owned(conn, &item.user_id, item.id).map_err(|error| match error {
                    ManagedWorkflowRepoError::Repository(RepoError::Query(error)) => error,
                    _ => diesel::result::Error::RollbackTransaction,
                })?;
            terminalize(
                conn,
                &item.user_id,
                &current,
                None,
                "service_restarted",
                "recovery",
                Some(
                    json!({"code":"service_restarted","message":"服务重启，研究工作流已中止"})
                        .to_string(),
                ),
                now_ms,
                &[],
            )
            .map_err(|error| match error {
                ManagedWorkflowRepoError::Repository(RepoError::Query(error)) => error,
                _ => diesel::result::Error::RollbackTransaction,
            })
        })?;
    }
    Ok(())
}

fn freeze_inputs(
    conn: &mut SqliteConnection,
    user: &str,
    workflow_id: i32,
    prepared: &mut PreparedStudyConfirmation,
    overlays: &[WorkflowStructOverlayMaterial],
    now_ms: i64,
) -> Result<(), ManagedWorkflowRepoError> {
    for prepared_overlay in &prepared.struct_overlays {
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
    if let Some(model) = &prepared.inputs.model {
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
    for resource in &mut prepared.inputs.resources {
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
            let prepared_overlay = prepared
                .struct_overlays
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
        let prepared_overlay = prepared
            .struct_overlays
            .iter()
            .find(|prepared| prepared.input_id == overlay.input_id)
            .ok_or(ManagedWorkflowRepoError::ConfigurationStale)?;
        if prepared_overlay.source_resource_id.is_some()
            || prepared
                .inputs
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
        prepared
            .inputs
            .resources
            .push(crate::application::FrozenStudyInput {
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
    let source = load_owned(conn, user, source_workflow_id)?;
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
        return load_owned(conn, user, existing);
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
    load_owned(conn, user, workflow_id)
}

fn validate_active_model(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
    prepared: &PreparedStudyConfirmation,
) -> Result<(), ManagedWorkflowRepoError> {
    let Some(expected) = prepared.inputs.model.as_ref() else {
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

#[allow(clippy::too_many_arguments)]
fn insert_input(
    conn: &mut SqliteConnection,
    workflow_id: i32,
    input_name: &str,
    input_kind: &str,
    resource_id: Option<&str>,
    model_version_id: Option<i32>,
    artifact_id: i32,
    input_alias: Option<&str>,
    schema_ref: Option<&str>,
    schema_hash: Option<&str>,
    content_hash: &str,
    snapshot: serde_json::Value,
    now_ms: i64,
) -> Result<(), ManagedWorkflowRepoError> {
    diesel::insert_into(schema::workflow_input_snapshots::table)
        .values((
            schema::workflow_input_snapshots::workflow_id.eq(workflow_id),
            schema::workflow_input_snapshots::input_name.eq(input_name),
            schema::workflow_input_snapshots::input_kind.eq(input_kind),
            schema::workflow_input_snapshots::resource_id.eq(resource_id),
            schema::workflow_input_snapshots::model_version_id.eq(model_version_id),
            schema::workflow_input_snapshots::artifact_id.eq(Some(artifact_id)),
            schema::workflow_input_snapshots::input_alias.eq(input_alias),
            schema::workflow_input_snapshots::schema_ref.eq(schema_ref),
            schema::workflow_input_snapshots::schema_hash.eq(schema_hash),
            schema::workflow_input_snapshots::content_hash.eq(Some(content_hash)),
            schema::workflow_input_snapshots::snapshot_json.eq(snapshot.to_string()),
            schema::workflow_input_snapshots::created_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    let owner = schema::managed_workflows::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::managed_workflows::session_id)),
        )
        .filter(schema::managed_workflows::id.eq(workflow_id))
        .select(schema::sessions::user_id)
        .first::<String>(conn)?;
    let scope_id = super::artifact_refs::storage_scope_id_for_user(conn, &UserId(owner))
        .map_err(configuration_artifact_error)?;
    super::artifact_refs::retain_ref_count(conn, scope_id, artifact_id, 1)
        .map_err(configuration_artifact_error)?;
    Ok(())
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

fn load_owned(
    conn: &mut SqliteConnection,
    user: &str,
    workflow_id: i32,
) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
    let workflow = schema::managed_workflows::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::managed_workflows::session_id)),
        )
        .filter(schema::managed_workflows::id.eq(workflow_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::managed_workflows::all_columns)
        .first::<ManagedWorkflowRecord>(conn)
        .optional()?
        .ok_or(ManagedWorkflowRepoError::Repository(RepoError::NotFound))?;
    let run = schema::simulation_runs::table
        .filter(schema::simulation_runs::managed_workflow_id.eq(workflow_id))
        .first::<SimulationRun>(conn)?;
    let attempts = schema::workflow_attempts::table
        .filter(schema::workflow_attempts::workflow_id.eq(workflow_id))
        .order(schema::workflow_attempts::attempt_number.asc())
        .load::<WorkflowAttemptRecord>(conn)?;
    let inputs = schema::workflow_input_snapshots::table
        .filter(schema::workflow_input_snapshots::workflow_id.eq(workflow_id))
        .order(schema::workflow_input_snapshots::id.asc())
        .load::<WorkflowInputSnapshotRecord>(conn)?;
    let output_dataset_ids = schema::result_datasets::table
        .filter(schema::result_datasets::run_id.eq(run.id))
        .filter(schema::result_datasets::deleted_at_ms.is_null())
        .order(schema::result_datasets::id.asc())
        .select(schema::result_datasets::id)
        .load::<i32>(conn)?;
    let progress_message_revision = workflow
        .progress_message_id
        .map(|id| {
            schema::messages::table
                .find(id)
                .select(schema::messages::revision)
                .first::<i32>(conn)
        })
        .transpose()?
        .unwrap_or(1);
    Ok(ManagedWorkflowAggregate {
        workflow,
        run,
        attempts,
        inputs,
        output_dataset_ids,
        progress_message_revision,
    })
}

fn last_id(conn: &mut SqliteConnection) -> RepoResult<i32> {
    diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result(conn)
    .map_err(RepoError::from)
}

async fn owned_scope_id(pool: &DbPool, user: &str) -> Result<i32, ManagedWorkflowRepoError> {
    let pool = pool.clone();
    let user = UserId(user.to_owned());
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(RepoError::Pool)?;
        super::artifact_refs::storage_scope_id_for_user(&mut conn, &user)
    })
    .await
    .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    .map_err(ManagedWorkflowRepoError::Repository)
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

fn configuration_artifact_error(error: RepoError) -> ManagedWorkflowRepoError {
    match error {
        RepoError::NotFound => ManagedWorkflowRepoError::ConfigurationStale,
        error => ManagedWorkflowRepoError::Repository(error),
    }
}
