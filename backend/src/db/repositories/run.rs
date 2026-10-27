use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};

use super::{RepoError, RepoResult, StateTransition};
use crate::auth::UserId;
use crate::db::models::SimulationRun;
use crate::db::{DbPool, schema};

#[derive(Debug, Clone)]
pub struct CreateRun {
    pub script_id: i32,
    pub model_version_id: i32,
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

#[derive(QueryableByName, Debug, Clone)]
pub struct RunModelFrame {
    #[diesel(sql_type=Text)]
    pub table_name: String,
    #[diesel(sql_type=Integer)]
    pub artifact_id: i32,
    #[diesel(sql_type=Text)]
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub run: SimulationRun,
    pub frames: Vec<RunModelFrame>,
}

#[derive(Debug)]
pub enum RunRepoError {
    Repository(RepoError),
    ActiveModelChanged,
    ScriptInvalid,
    ScriptIncompatible(Vec<String>),
    IdempotencyConflict,
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
                    if existing.script_resource_id != request.script_id
                        || existing.model_version_id != Some(request.model_version_id)
                    {
                        return Err(RunRepoError::IdempotencyConflict);
                    }
                    return Ok(CreatedRun {
                        run: existing,
                        replayed: true,
                    });
                }
                #[derive(QueryableByName)]
                struct Active {
                    #[diesel(sql_type=Nullable<Integer>)]
                    active_version_id: Option<i32>,
                }
                let active = diesel::sql_query(
                    "SELECT active_version_id FROM workspaces WHERE session_id=?1",
                )
                .bind::<Integer, _>(script.session_id)
                .get_result::<Active>(conn)
                .optional()?
                .and_then(|row| row.active_version_id);
                if active != Some(request.model_version_id) {
                    return Err(RunRepoError::ActiveModelChanged);
                }
                #[derive(QueryableByName)]
                struct Version {
                    #[diesel(sql_type=Text)]
                    content_hash: String,
                }
                let version = diesel::sql_query(
                    "SELECT content_hash FROM model_versions WHERE id=?1 AND deleted_at_ms IS NULL",
                )
                .bind::<Integer, _>(request.model_version_id)
                .get_result::<Version>(conn)
                .optional()?
                .ok_or(RunRepoError::ActiveModelChanged)?;
                if script.validation_status != "valid" {
                    return Err(RunRepoError::ScriptInvalid);
                }
                #[derive(QueryableByName)]
                struct Alias {
                    #[diesel(sql_type=Text)]
                    table_name: String,
                }
                let aliases = diesel::sql_query(
                    "SELECT table_name FROM model_dataframes WHERE version_id=?1",
                )
                .bind::<Integer, _>(request.model_version_id)
                .load::<Alias>(conn)?
                .into_iter()
                .map(|row| row.table_name)
                .collect::<Vec<_>>();
                let required: Vec<String> =
                    serde_json::from_str(&script.required_aliases_json).unwrap_or_default();
                let missing = required
                    .into_iter()
                    .filter(|name| !aliases.contains(name))
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    return Err(RunRepoError::ScriptIncompatible(missing));
                }
                if aliases.iter().any(|alias| !valid_alias(alias)) {
                    return Err(RunRepoError::ScriptIncompatible(vec![
                        "invalid_model_alias".into(),
                    ]));
                }
                let scope_id = schema::users::table
                    .find(&user)
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                let retained = diesel::update(
                    schema::storage_scope_artifacts::table.find((scope_id, script.artifact_id)),
                )
                .set(
                    schema::storage_scope_artifacts::ref_count
                        .eq(schema::storage_scope_artifacts::ref_count + 1),
                )
                .execute(conn)?;
                if retained != 1 {
                    return Err(RunRepoError::Repository(RepoError::NotFound));
                }
                diesel::insert_into(schema::simulation_runs::table)
                    .values((
                        schema::simulation_runs::session_id.eq(script.session_id),
                        schema::simulation_runs::model_version_id.eq(request.model_version_id),
                        schema::simulation_runs::model_content_hash.eq(version.content_hash),
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
        let pool = self.pool.clone();
        let version_id = run.model_version_id;
        let frames = tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let Some(version_id) = version_id else {
                return Ok(Vec::new());
            };
            diesel::sql_query(
                "SELECT table_name,artifact_id,content_hash FROM model_dataframes \
                 WHERE version_id=?1 ORDER BY table_name",
            )
            .bind::<Integer, _>(version_id)
            .load::<RunModelFrame>(&mut conn)
            .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)??;
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
}

fn valid_alias(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
