use diesel::prelude::*;
use serde_json::json;
use sha2::Digest;

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
pub struct ManagedWorkflowStartCommand {
    pub configuration_id: i32,
    pub session_id: i32,
    pub configuration_revision: i64,
    pub engine_snapshot_id: String,
    pub facade_contract_hash: String,
    pub locked_contract_json: String,
    pub frozen_inputs_json: String,
    pub struct_overlays_json: String,
    pub result_contract_json: String,
    pub client_operation_id: String,
    pub correction_budget: i32,
    pub agent_bundle_version: String,
    pub agent_skill_refs_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenStartModel {
    model_version_id: i32,
    content_hash: String,
    tables: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenStartResource {
    input_id: String,
    alias: String,
    resource_alias: String,
    kind: String,
    resource_id: Option<String>,
    schema_ref: String,
    schema_hash: String,
    content_hash: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenStartInputs {
    model: Option<FrozenStartModel>,
    resources: Vec<FrozenStartResource>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartStructOverlay {
    input_id: String,
    input_alias: String,
    source_resource_id: Option<String>,
    schema_ref: String,
    effective_values: serde_json::Value,
    user_overrides: serde_json::Value,
    selector_overrides: serde_json::Value,
    overrides: serde_json::Value,
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
            load_owned_on(&mut conn, &user, workflow_id)
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }
}

include!("managed_workflow_confirmation.rs");
include!("managed_workflow_retry.rs");
include!("managed_workflow_attempt.rs");
include!("managed_workflow_recovery.rs");

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

pub(crate) fn load_owned_on(
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
