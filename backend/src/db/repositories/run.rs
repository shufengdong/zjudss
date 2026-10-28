use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use sha2::Digest;

use super::{ArtifactDraft, ArtifactRepo, RepoError, RepoResult, StateTransition};
use crate::auth::UserId;
use crate::db::models::SimulationRun;
use crate::db::{DbPool, schema};

#[derive(Debug, Clone)]
pub struct CreateRun {
    pub script_id: i32,
    pub client_run_id: String,
    pub engine_contract_version: String,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone)]
pub struct CreatedRun {
    pub run: SimulationRun,
    pub replayed: bool,
}

#[derive(QueryableByName, Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDataset {
    #[diesel(sql_type=Integer)]
    pub id: i32,
    #[diesel(sql_type=Nullable<Integer>)]
    pub run_id: Option<i32>,
    #[diesel(sql_type=Nullable<Integer>)]
    pub analysis_run_id: Option<i32>,
    #[diesel(sql_type=Nullable<Integer>)]
    pub artifact_id: Option<i32>,
    #[diesel(sql_type=Text)]
    pub result_name: String,
    #[diesel(sql_type=Text)]
    pub schema_json: String,
    #[diesel(sql_type=Text)]
    pub schema_hash: String,
    #[diesel(sql_type=BigInt)]
    pub row_count: i64,
    #[diesel(sql_type=BigInt)]
    pub size_bytes: i64,
    #[diesel(sql_type=BigInt)]
    pub created_at_ms: i64,
    #[diesel(sql_type=Nullable<BigInt>)]
    pub deleted_at_ms: Option<i64>,
    #[diesel(sql_type=Nullable<Text>)]
    pub tombstone_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewRunDataset {
    pub artifact_id: i32,
    pub result_name: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct RunArtifactOutputs {
    pub scope_id: i32,
    pub run_id: i32,
    pub drafts: Vec<ArtifactDraft>,
    pub datasets: Vec<NewRunDataset>,
    pub finished_at_ms: i64,
}

#[derive(QueryableByName, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunModelFrame {
    #[diesel(sql_type=Text)]
    pub table_name: String,
    #[diesel(sql_type=Integer)]
    pub artifact_id: i32,
    #[diesel(sql_type=Text)]
    pub content_hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FrozenRunInputs {
    model: Option<FrozenRunModel>,
    #[serde(default)]
    resources: Vec<FrozenRunResource>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrozenRunModel {
    model_version_id: i32,
    content_hash: String,
    frames: Vec<RunModelFrame>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrozenRunResource {
    #[serde(default)]
    input_name: Option<String>,
    #[serde(default)]
    input_kind: Option<String>,
    resource_id: String,
    alias: String,
    artifact_id: i32,
    #[serde(default)]
    schema_ref: Option<String>,
    schema_hash: String,
    content_hash: String,
    #[serde(default)]
    source_kind: String,
    #[serde(default)]
    lineage: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub run: SimulationRun,
    pub frames: Vec<RunModelFrame>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDeletionImpact {
    pub status: String,
    pub result_count: i64,
    pub result_bytes: i64,
    pub derived_reference_count: i64,
    pub promoted_reference_count: i64,
    pub result_ids: Vec<i32>,
}

#[derive(Debug)]
pub enum RunRepoError {
    Repository(RepoError),
    ActiveModelChanged,
    ScriptInvalid,
    ScriptIncompatible(Vec<String>),
    IdempotencyConflict,
    RunBusy,
    ResultBusy,
    InvalidDeletionToken,
    DeletionImpactChanged,
}
impl From<RepoError> for RunRepoError {
    fn from(v: RepoError) -> Self {
        Self::Repository(v)
    }
}
impl From<diesel::result::Error> for RunRepoError {
    fn from(v: diesel::result::Error) -> Self {
        Self::Repository(RepoError::Query(v))
    }
}

#[derive(Clone)]
pub struct RunRepo {
    pool: DbPool,
}
impl RunRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        user: &UserId,
        request: CreateRun,
    ) -> Result<CreatedRun, RunRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<CreatedRun, RunRepoError, _>(|conn| {
                let script = schema::script_resources::table
                    .inner_join(
                        schema::sessions::table
                            .on(schema::sessions::id.eq(schema::script_resources::session_id)),
                    )
                    .filter(schema::script_resources::id.eq(request.script_id))
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::script_resources::all_columns)
                    .first::<crate::db::models::ScriptResource>(conn)
                    .optional()?
                    .ok_or(RunRepoError::Repository(RepoError::NotFound))?;
                if let Some(existing) = schema::simulation_runs::table
                    .filter(schema::simulation_runs::session_id.eq(script.session_id))
                    .filter(schema::simulation_runs::client_run_id.eq(&request.client_run_id))
                    .first::<SimulationRun>(conn)
                    .optional()?
                {
                    if existing.script_resource_id != Some(request.script_id) {
                        return Err(RunRepoError::IdempotencyConflict);
                    }
                    return Ok(CreatedRun {
                        run: existing,
                        replayed: true,
                    });
                }
                #[derive(QueryableByName)]
                struct ActiveVersion {
                    #[diesel(sql_type=Integer)]
                    version_id: i32,
                    #[diesel(sql_type=Text)]
                    content_hash: String,
                }
                if script.validation_status != "valid" {
                    return Err(RunRepoError::ScriptInvalid);
                }
                let managed = script.source_kind == "agent" && script.managed_workflow_id.is_some();
                let required_model = if let Some(workflow_id) = script.managed_workflow_id {
                    diesel::sql_query("SELECT input_alias table_name,artifact_id,content_hash FROM workflow_input_snapshots WHERE workflow_id=?1 AND input_kind='model' ORDER BY input_name")
                        .bind::<Integer, _>(workflow_id)
                        .load::<RunModelFrame>(conn)?
                        .into_iter().map(|frame| frame.table_name).collect::<Vec<_>>()
                } else { Vec::new() };
                let active = diesel::sql_query("SELECT v.id version_id,v.content_hash FROM workspaces w JOIN model_versions v ON v.id=w.active_version_id WHERE w.session_id=?1 AND v.deleted_at_ms IS NULL")
                    .bind::<Integer, _>(script.session_id)
                    .get_result::<ActiveVersion>(conn).optional()?;
                let model = if managed && required_model.is_empty() {
                    None
                } else if let Some(active) = active {
                    let all_frames = diesel::sql_query("SELECT table_name,artifact_id,content_hash FROM model_dataframes WHERE version_id=?1 ORDER BY table_name")
                        .bind::<Integer, _>(active.version_id)
                        .load::<RunModelFrame>(conn)?;
                    let frames = if managed {
                        let missing = required_model.iter().filter(|name| !all_frames.iter().any(|frame| &frame.table_name == *name)).cloned().collect::<Vec<_>>();
                        if !missing.is_empty() { return Err(RunRepoError::ScriptIncompatible(missing)); }
                        all_frames.into_iter().filter(|frame| required_model.contains(&frame.table_name)).collect()
                    } else { all_frames };
                    Some(FrozenRunModel { model_version_id: active.version_id, content_hash: active.content_hash, frames })
                } else if managed {
                    return Err(RunRepoError::ActiveModelChanged);
                } else {
                    None
                };
                #[derive(QueryableByName)]
                struct ResourceRow {
                    #[diesel(sql_type=Nullable<Text>)] input_name: Option<String>,
                    #[diesel(sql_type=Nullable<Text>)] input_kind: Option<String>,
                    #[diesel(sql_type=Text)] resource_id: String,
                    #[diesel(sql_type=Text)] alias: String,
                    #[diesel(sql_type=Integer)] artifact_id: i32,
                    #[diesel(sql_type=Nullable<Text>)] expected_schema_ref: Option<String>,
                    #[diesel(sql_type=Nullable<Text>)] current_schema_ref: Option<String>,
                    #[diesel(sql_type=Text)] schema_hash: String,
                    #[diesel(sql_type=Text)] content_hash: String,
                    #[diesel(sql_type=Text)] source_kind: String,
                    #[diesel(sql_type=Text)] lineage_json: String,
                }
                let resource_rows = if managed {
                    let workflow_id = script.managed_workflow_id.expect("managed workflow");
                    diesel::sql_query("SELECT i.input_name,i.input_kind,d.id resource_id,d.alias,d.normalized_artifact_id artifact_id,i.schema_ref expected_schema_ref,d.declared_schema_ref current_schema_ref,d.schema_hash,a.sha256 content_hash,d.source_kind,d.lineage_json FROM workflow_input_snapshots i JOIN data_resources d ON lower(d.alias)=lower(i.input_alias) AND d.session_id=?1 AND d.status='live' JOIN artifacts a ON a.id=d.normalized_artifact_id WHERE i.workflow_id=?2 AND i.input_kind<>'model' ORDER BY i.input_name")
                        .bind::<Integer, _>(script.session_id).bind::<Integer, _>(workflow_id).load::<ResourceRow>(conn)?
                } else {
                    diesel::sql_query("SELECT NULL input_name,NULL input_kind,d.id resource_id,d.alias,d.normalized_artifact_id artifact_id,NULL expected_schema_ref,d.declared_schema_ref current_schema_ref,d.schema_hash,a.sha256 content_hash,d.source_kind,d.lineage_json FROM data_resources d JOIN artifacts a ON a.id=d.normalized_artifact_id WHERE d.session_id=?1 AND d.status='live' ORDER BY d.alias")
                        .bind::<Integer, _>(script.session_id).load::<ResourceRow>(conn)?
                };
                if managed {
                    #[derive(QueryableByName)] struct Count { #[diesel(sql_type=BigInt)] count: i64 }
                    let expected = diesel::sql_query("SELECT count(*) count FROM workflow_input_snapshots WHERE workflow_id=?1 AND input_kind<>'model'")
                        .bind::<Integer, _>(script.managed_workflow_id.expect("managed workflow")).get_result::<Count>(conn)?.count;
                    let mut incompatible = resource_rows.iter().filter(|row| row.expected_schema_ref != row.current_schema_ref).filter_map(|row| row.input_name.clone()).collect::<Vec<_>>();
                    if resource_rows.len() as i64 != expected { incompatible.push("missing_data_resource".into()); }
                    if !incompatible.is_empty() { return Err(RunRepoError::ScriptIncompatible(incompatible)); }
                }
                let resources = resource_rows.into_iter().map(|row| FrozenRunResource {
                    input_name: row.input_name,
                    input_kind: row.input_kind,
                    resource_id: row.resource_id,
                    alias: row.alias,
                    artifact_id: row.artifact_id,
                    schema_ref: row.current_schema_ref,
                    schema_hash: row.schema_hash,
                    content_hash: row.content_hash,
                    source_kind: row.source_kind,
                    lineage: serde_json::from_str(&row.lineage_json).unwrap_or(serde_json::Value::Null),
                }).collect::<Vec<_>>();
                let frozen = FrozenRunInputs { model, resources };
                let input_snapshot_json = serde_json::to_string(&frozen).map_err(|error| RunRepoError::Repository(RepoError::Storage(error.to_string())))?;
                let scope_id = schema::users::table
                    .find(&user)
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                let mut retained_artifacts = vec![script.artifact_id];
                retained_artifacts.extend(frozen.model.iter().flat_map(|model| model.frames.iter().map(|frame| frame.artifact_id)));
                retained_artifacts.extend(frozen.resources.iter().map(|resource| resource.artifact_id));
                for artifact_id in retained_artifacts {
                    let retained = diesel::update(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
                        .set(schema::storage_scope_artifacts::ref_count.eq(schema::storage_scope_artifacts::ref_count + 1)).execute(conn)?;
                    if retained != 1 { return Err(RunRepoError::Repository(RepoError::NotFound)); }
                }
                #[derive(QueryableByName)] struct MessageId { #[diesel(sql_type=Integer)] id: i32 }
                let script_message_id = diesel::sql_query("SELECT id FROM messages WHERE session_id=?1 AND kind='script' AND json_extract(payload_json,'$.script.id')=CAST(?2 AS TEXT) ORDER BY id DESC LIMIT 1")
                    .bind::<Integer, _>(script.session_id).bind::<Integer, _>(script.id).get_result::<MessageId>(conn).optional()?.map(|row| row.id);
                let result_contract_json = if managed {
                    let contract = serde_json::from_str::<serde_json::Value>(script.frozen_contract_json.as_deref().ok_or_else(|| RepoError::InvalidState("managed script contract missing".into()))?)
                        .map_err(|error| RepoError::Storage(format!("invalid managed script contract: {error}")))?;
                    Some(contract.get("results").ok_or_else(|| RepoError::InvalidState("managed script result contract missing".into()))?.to_string())
                } else { None };
                diesel::insert_into(schema::simulation_runs::table)
                    .values((
                        schema::simulation_runs::session_id.eq(script.session_id),
                        schema::simulation_runs::model_version_id.eq(frozen.model.as_ref().map(|model| model.model_version_id)),
                        schema::simulation_runs::model_content_hash.eq(frozen.model.as_ref().map(|model| model.content_hash.clone())),
                        schema::simulation_runs::script_resource_id.eq(script.id),
                        schema::simulation_runs::script_artifact_id.eq(script.artifact_id),
                        schema::simulation_runs::script_content_hash.eq(script.content_hash),
                        schema::simulation_runs::client_run_id.eq(request.client_run_id),
                        schema::simulation_runs::status.eq("queued"),
                        schema::simulation_runs::stage.eq("queued"),
                        schema::simulation_runs::engine_contract_version
                            .eq(request.engine_contract_version),
                        schema::simulation_runs::agent_bundle_version
                            .eq(script.agent_bundle_version),
                        schema::simulation_runs::agent_skill_refs_json
                            .eq(script.agent_skill_refs_json),
                        schema::simulation_runs::script_message_id.eq(script_message_id),
                        schema::simulation_runs::run_kind.eq(if managed { "agent_rerun" } else { "manual" }),
                        schema::simulation_runs::input_snapshot_json.eq(input_snapshot_json),
                        schema::simulation_runs::result_contract_json.eq(result_contract_json),
                        schema::simulation_runs::created_at_ms.eq(request.created_at_ms),
                    ))
                    .execute(conn)?;
                let id: i32 = diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()"))
                    .get_result(conn)?;
                Ok(CreatedRun {
                    run: schema::simulation_runs::table.find(id).first(conn)?,
                    replayed: false,
                })
            })
        })
        .await
        .map_err(|error| RunRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<SimulationRun> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::simulation_runs::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::simulation_runs::session_id)),
                )
                .filter(schema::simulation_runs::id.eq(id))
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .filter(schema::simulation_runs::deleted_at_ms.is_null())
                .select(schema::simulation_runs::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn list(
        &self,
        user: &UserId,
        session_id: i32,
        limit: i64,
    ) -> RepoResult<Vec<SimulationRun>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .find(session_id)
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            schema::simulation_runs::table
                .filter(schema::simulation_runs::session_id.eq(session_id))
                .filter(schema::simulation_runs::deleted_at_ms.is_null())
                .order(schema::simulation_runs::id.desc())
                .limit(limit.clamp(1, 100))
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn context(&self, user: &UserId, id: i32) -> RepoResult<RunContext> {
        let run = self.find(user, id).await?;
        let snapshot: FrozenRunInputs = serde_json::from_str(&run.input_snapshot_json)
            .map_err(|error| RepoError::Storage(format!("invalid run input snapshot: {error}")))?;
        let mut frames = snapshot.model.map(|model| model.frames).unwrap_or_default();
        frames.extend(
            snapshot
                .resources
                .into_iter()
                .map(|resource| RunModelFrame {
                    table_name: resource.alias,
                    artifact_id: resource.artifact_id,
                    content_hash: resource.content_hash,
                }),
        );
        Ok(RunContext { run, frames })
    }
    pub async fn set_running(
        &self,
        user: &UserId,
        id: i32,
        stage: &str,
        now: i64,
    ) -> RepoResult<()> {
        self.update_state(user, id, StateTransition::running(stage, now))
            .await
    }
    pub async fn fail(
        &self,
        user: &UserId,
        id: i32,
        stage: &str,
        error: String,
        exit_code: Option<i32>,
        now: i64,
    ) -> RepoResult<()> {
        self.update_state(
            user,
            id,
            StateTransition::failed_with_exit_code(stage, error, exit_code, now),
        )
        .await
    }
    async fn update_state(
        &self,
        user: &UserId,
        id: i32,
        transition: StateTransition,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let count = diesel::sql_query(
                "UPDATE simulation_runs SET status=?1,stage=?2,error_json=?3,\
                 exit_code=?4,started_at_ms=COALESCE(started_at_ms,?5),finished_at_ms=?6 \
                 WHERE id=?7 AND deleted_at_ms IS NULL AND \
                 session_id IN(SELECT id FROM sessions WHERE user_id=?8)",
            )
            .bind::<Text, _>(transition.status)
            .bind::<Text, _>(transition.stage)
            .bind::<Nullable<Text>, _>(transition.error)
            .bind::<Nullable<Integer>, _>(transition.exit_code)
            .bind::<Nullable<BigInt>, _>(transition.started_at_ms)
            .bind::<Nullable<BigInt>, _>(transition.finished_at_ms)
            .bind::<Integer, _>(id)
            .bind::<Text, _>(user)
            .execute(&mut conn)?;
            if count == 1 {
                Ok(())
            } else {
                Err(RepoError::NotFound)
            }
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn complete(
        &self,
        user: &UserId,
        id: i32,
        datasets: Vec<NewRunDataset>,
        now: i64,
    ) -> RepoResult<Vec<RunDataset>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Vec<RunDataset>, RepoError, _>(|conn| {
                let owned_sessions = schema::sessions::table
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::sessions::id);
                let exists = schema::simulation_runs::table
                    .filter(schema::simulation_runs::id.eq(id))
                    .filter(schema::simulation_runs::session_id.eq_any(owned_sessions))
                    .filter(schema::simulation_runs::deleted_at_ms.is_null())
                    .select(schema::simulation_runs::id)
                    .first::<i32>(conn)
                    .optional()?;
                if exists.is_none() {
                    return Err(RepoError::NotFound);
                }
                for dataset in datasets {
                    diesel::sql_query("INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")
                        .bind::<Integer, _>(id)
                        .bind::<Integer, _>(dataset.artifact_id)
                        .bind::<Text, _>(dataset.result_name)
                        .bind::<Text, _>(dataset.schema_json)
                        .bind::<Text, _>(dataset.schema_hash)
                        .bind::<BigInt, _>(dataset.row_count)
                        .bind::<BigInt, _>(dataset.size_bytes)
                        .bind::<BigInt, _>(dataset.created_at_ms)
                        .execute(conn)?;
                }
                diesel::sql_query("UPDATE simulation_runs SET status='succeeded',stage='completed',exit_code=0,error_json=NULL,finished_at_ms=?1 WHERE id=?2")
                    .bind::<BigInt, _>(now)
                    .bind::<Integer, _>(id)
                    .execute(conn)?;
                diesel::sql_query("SELECT * FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL ORDER BY id")
                    .bind::<Integer, _>(id)
                    .load(conn)
                    .map_err(RepoError::from)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub(crate) async fn complete_with_artifact_outputs<P>(
        &self,
        user: &UserId,
        outputs: RunArtifactOutputs,
        publish: P,
    ) -> RepoResult<Vec<RunDataset>>
    where
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
    {
        let user = user.0.clone();
        let RunArtifactOutputs {
            scope_id,
            run_id,
            drafts,
            datasets,
            finished_at_ms,
        } = outputs;
        ArtifactRepo::new(self.pool.clone())
            .commit_batch_with_publish(scope_id, drafts, publish, move |conn, artifacts| {
                if artifacts.len() != datasets.len() { return Err(RepoError::InvalidState("result artifact count mismatch".into())); }
                let owned = schema::simulation_runs::table
                    .inner_join(schema::sessions::table.on(schema::sessions::id.eq(schema::simulation_runs::session_id)))
                    .filter(schema::simulation_runs::id.eq(run_id)).filter(schema::sessions::user_id.eq(&user)).filter(schema::sessions::deletion_status.eq("live")).filter(schema::simulation_runs::deleted_at_ms.is_null()).select(schema::simulation_runs::id).first::<i32>(conn).optional()?;
                if owned.is_none() { return Err(RepoError::NotFound); }
                for (dataset, artifact) in datasets.into_iter().zip(artifacts) {
                    diesel::sql_query("INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")
                        .bind::<Integer,_>(run_id).bind::<Integer,_>(artifact.id).bind::<Text,_>(dataset.result_name).bind::<Text,_>(dataset.schema_json).bind::<Text,_>(dataset.schema_hash).bind::<BigInt,_>(dataset.row_count).bind::<BigInt,_>(dataset.size_bytes).bind::<BigInt,_>(dataset.created_at_ms).execute(conn)?;
                }
                diesel::sql_query("UPDATE simulation_runs SET status='succeeded',stage='completed',exit_code=0,error_json=NULL,finished_at_ms=?1 WHERE id=?2")
                    .bind::<BigInt,_>(finished_at_ms).bind::<Integer,_>(run_id).execute(conn)?;
                diesel::sql_query("SELECT * FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL ORDER BY id").bind::<Integer,_>(run_id).load(conn).map_err(RepoError::from)
            })
            .await
    }
    pub async fn datasets(&self, user: &UserId, id: i32) -> RepoResult<Vec<RunDataset>> {
        self.find(user, id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query(
                "SELECT * FROM result_datasets \
                 WHERE run_id=?1 AND deleted_at_ms IS NULL ORDER BY id",
            )
            .bind::<Integer, _>(id)
            .load(&mut conn)
            .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn latest_statuses(
        &self,
        user: &UserId,
    ) -> RepoResult<std::collections::HashMap<i32, String>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            #[derive(QueryableByName)]
            struct Row {
                #[diesel(sql_type = Integer)]
                session_id: i32,
                #[diesel(sql_type = Text)]
                status: String,
            }

            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let rows = diesel::sql_query(
                "SELECT r.session_id,r.status FROM simulation_runs r \
                 JOIN sessions s ON s.id=r.session_id \
                 WHERE s.user_id=?1 AND r.deleted_at_ms IS NULL AND \
                 r.id IN(SELECT max(r2.id) FROM simulation_runs r2 \
                 WHERE r2.deleted_at_ms IS NULL GROUP BY r2.session_id)",
            )
            .bind::<Text, _>(user)
            .load::<Row>(&mut conn)?;
            Ok(rows
                .into_iter()
                .map(|row| (row.session_id, row.status))
                .collect())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn preview_deletion(
        &self,
        user: &UserId,
        run_id: i32,
        token_hash: String,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<RunDeletionImpact, RunRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<RunDeletionImpact, RunRepoError, _>(|conn| {
                let run = scoped_run(conn, &user, run_id, true)?;
                if matches!(run.status.as_str(), "queued" | "running") {
                    return Err(RunRepoError::RunBusy);
                }
                let impact = run_deletion_impact(conn, &run)?;
                let impact_json = serde_json::to_string(&impact).map_err(|error| RepoError::Storage(error.to_string()))?;
                let impact_hash = hex::encode(sha2::Sha256::digest(impact_json.as_bytes()));
                diesel::sql_query("DELETE FROM run_deletion_previews WHERE expires_at_ms<?1 OR (user_id=?2 AND run_id=?3)")
                    .bind::<BigInt,_>(now_ms).bind::<Text,_>(&user).bind::<Integer,_>(run_id).execute(conn)?;
                diesel::sql_query("INSERT INTO run_deletion_previews(token_hash,user_id,run_id,impact_hash,impact_json,expires_at_ms,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                    .bind::<Text,_>(token_hash).bind::<Text,_>(user).bind::<Integer,_>(run_id).bind::<Text,_>(impact_hash).bind::<Text,_>(impact_json).bind::<BigInt,_>(expires_at_ms).bind::<BigInt,_>(now_ms).execute(conn)?;
                Ok(impact)
            })
        }).await.map_err(|error| RunRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn delete_run(
        &self,
        user: &UserId,
        run_id: i32,
        token_hash: String,
        now_ms: i64,
    ) -> Result<(), RunRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(), RunRepoError, _>(|conn| {
                #[derive(QueryableByName)] struct Preview { #[diesel(sql_type=Text)] impact_hash: String }
                let preview = diesel::sql_query("SELECT impact_hash FROM run_deletion_previews WHERE token_hash=?1 AND user_id=?2 AND run_id=?3 AND expires_at_ms>=?4")
                    .bind::<Text,_>(&token_hash).bind::<Text,_>(&user).bind::<Integer,_>(run_id).bind::<BigInt,_>(now_ms).get_result::<Preview>(conn).optional()?
                    .ok_or(RunRepoError::InvalidDeletionToken)?;
                let run = scoped_run(conn, &user, run_id, false)?;
                if run.deleted_at_ms.is_some() { return Ok(()); }
                if matches!(run.status.as_str(), "queued" | "running") { return Err(RunRepoError::RunBusy); }
                let impact = run_deletion_impact(conn, &run)?;
                let impact_json = serde_json::to_string(&impact).map_err(|error| RepoError::Storage(error.to_string()))?;
                let actual_hash = hex::encode(sha2::Sha256::digest(impact_json.as_bytes()));
                if actual_hash != preview.impact_hash { return Err(RunRepoError::DeletionImpactChanged); }
                #[derive(QueryableByName)] struct Count { #[diesel(sql_type=BigInt)] count: i64 }
                let busy = diesel::sql_query("SELECT count(*) count FROM result_analysis_inputs i JOIN result_analysis_runs a ON a.id=i.analysis_run_id WHERE i.dataset_id IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL) AND a.status IN('pending','running')")
                    .bind::<Integer,_>(run_id).get_result::<Count>(conn)?.count;
                if busy > 0 { return Err(RunRepoError::ResultBusy); }
                #[derive(QueryableByName)] struct ArtifactId { #[diesel(sql_type=Integer)] artifact_id: i32 }
                let mut artifacts = diesel::sql_query("SELECT artifact_id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL AND artifact_id IS NOT NULL ORDER BY id")
                    .bind::<Integer,_>(run_id).load::<ArtifactId>(conn)?.into_iter().map(|row| row.artifact_id).collect::<Vec<_>>();
                if let Some(artifact_id) = run.script_artifact_id { artifacts.push(artifact_id); }
                let snapshot: FrozenRunInputs = serde_json::from_str(&run.input_snapshot_json)
                    .map_err(|error| RepoError::Storage(format!("invalid run input snapshot: {error}")))?;
                artifacts.extend(snapshot.model.iter().flat_map(|model| model.frames.iter().map(|frame| frame.artifact_id)));
                artifacts.extend(snapshot.resources.iter().map(|resource| resource.artifact_id));
                let scope_id = schema::users::table.find(&user).select(schema::users::storage_scope_id).first::<i32>(conn)?;
                super::artifact::release_refs(conn, scope_id, artifacts, now_ms)?;
                let tombstone = serde_json::json!({"clientRunId":run.client_run_id,"status":run.status,"modelContentHash":run.model_content_hash,"scriptContentHash":run.script_content_hash,"impact":impact}).to_string();
                diesel::sql_query("UPDATE simulation_runs SET deleted_at_ms=?1,tombstone_json=?2 WHERE id=?3 AND deleted_at_ms IS NULL")
                    .bind::<BigInt,_>(now_ms).bind::<Text,_>(tombstone).bind::<Integer,_>(run_id).execute(conn)?;
                Ok(())
            })
        }).await.map_err(|error| RunRepoError::Repository(RepoError::Join(error)))?
    }
}

fn scoped_run(
    conn: &mut SqliteConnection,
    user: &str,
    run_id: i32,
    live: bool,
) -> Result<SimulationRun, RunRepoError> {
    let mut query = schema::simulation_runs::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::simulation_runs::session_id)),
        )
        .filter(schema::simulation_runs::id.eq(run_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::simulation_runs::all_columns)
        .into_boxed();
    if live {
        query = query.filter(schema::simulation_runs::deleted_at_ms.is_null());
    }
    query
        .first(conn)
        .optional()?
        .ok_or(RunRepoError::Repository(RepoError::NotFound))
}

fn run_deletion_impact(
    conn: &mut SqliteConnection,
    run: &SimulationRun,
) -> Result<RunDeletionImpact, RunRepoError> {
    #[derive(QueryableByName)]
    struct ResultRow {
        #[diesel(sql_type=Integer)]
        id: i32,
        #[diesel(sql_type=BigInt)]
        size_bytes: i64,
    }
    let rows = diesel::sql_query("SELECT id,size_bytes FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL ORDER BY id")
        .bind::<Integer,_>(run.id).load::<ResultRow>(conn)?;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let bytes = rows.iter().map(|row| row.size_bytes).sum();
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type=BigInt)]
        count: i64,
    }
    let derived = diesel::sql_query("SELECT count(*) count FROM dataset_lineage_snapshots WHERE source_dataset_id IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL)")
        .bind::<Integer,_>(run.id).get_result::<Count>(conn)?.count;
    let promoted = diesel::sql_query("SELECT count(*) count FROM data_resources WHERE source_kind='result' AND status<>'deleted' AND CAST(json_extract(lineage_json,'$.datasetId') AS INTEGER) IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL)")
        .bind::<Integer,_>(run.id).get_result::<Count>(conn)?.count;
    Ok(RunDeletionImpact {
        status: run.status.clone(),
        result_count: rows.len() as i64,
        result_bytes: bytes,
        derived_reference_count: derived,
        promoted_reference_count: promoted,
        result_ids: ids,
    })
}
