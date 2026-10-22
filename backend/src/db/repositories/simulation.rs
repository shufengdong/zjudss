use diesel::prelude::*;

use crate::auth::AccessScope;
use crate::db::models::{NewResultDataset, NewV2SimulationTask, ResultDataset, SimulationTask};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct SimulationRepo {
    pool: DbPool,
}

impl SimulationRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        scope: &AccessScope,
        task: NewV2SimulationTask,
    ) -> RepoResult<SimulationTask> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction::<_, RepoError, _>(|conn| {
                let version_id = task
                    .model_version_id
                    .ok_or_else(|| RepoError::InvalidState("model_version_id 必填".into()))?;
                let script_id = task
                    .script_artifact_id
                    .ok_or_else(|| RepoError::InvalidState("script_artifact_id 必填".into()))?;
                let storage_scope_id = schema::sessions::table
                    .filter(schema::sessions::id.eq(task.session_id))
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::storage_scope_id)
                    .first::<Option<i32>>(conn)?
                    .ok_or(RepoError::NotFound)?;
                schema::model_versions::table
                    .inner_join(schema::workspaces::table)
                    .filter(schema::model_versions::id.eq(version_id))
                    .filter(schema::workspaces::session_id.eq(task.session_id))
                    .filter(schema::model_versions::status.eq("ready"))
                    .select(schema::model_versions::id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                schema::storage_scope_artifacts::table
                    .filter(schema::storage_scope_artifacts::storage_scope_id.eq(storage_scope_id))
                    .filter(schema::storage_scope_artifacts::artifact_id.eq(script_id))
                    .select(schema::storage_scope_artifacts::artifact_id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                diesel::insert_into(schema::simulation_tasks::table)
                    .values(task)
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                Ok(schema::simulation_tasks::table.find(id).first(conn)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_stage(
        &self,
        scope: &AccessScope,
        id: i32,
        stage: &str,
        started_at_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let count = diesel::update(
                schema::simulation_tasks::table
                    .filter(schema::simulation_tasks::id.eq(id))
                    .filter(schema::simulation_tasks::session_id.eq_any(sessions)),
            )
            .set((
                schema::simulation_tasks::status.eq("running"),
                schema::simulation_tasks::stage.eq(stage),
                schema::simulation_tasks::started_at_ms.eq(Some(started_at_ms)),
            ))
            .execute(&mut conn)?;
            if count == 0 {
                Err(RepoError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn complete(
        &self,
        scope: &AccessScope,
        id: i32,
        datasets: Vec<NewResultDataset>,
        now_ms: i64,
    ) -> RepoResult<Vec<ResultDataset>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction::<_, RepoError, _>(|conn| {
                let sessions = schema::sessions::table
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::id);
                let found = schema::simulation_tasks::table
                    .filter(schema::simulation_tasks::id.eq(id))
                    .filter(schema::simulation_tasks::session_id.eq_any(sessions))
                    .select(schema::simulation_tasks::id)
                    .first::<i32>(conn)
                    .optional()?;
                if found.is_none() {
                    return Err(RepoError::NotFound);
                }
                for dataset in datasets {
                    diesel::insert_into(schema::result_datasets::table)
                        .values(dataset)
                        .execute(conn)?;
                }
                diesel::update(schema::simulation_tasks::table.find(id))
                    .set((
                        schema::simulation_tasks::status.eq("succeeded"),
                        schema::simulation_tasks::stage.eq("collect"),
                        schema::simulation_tasks::exit_code.eq(0),
                        schema::simulation_tasks::error_json.eq::<Option<String>>(None),
                        schema::simulation_tasks::finished_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(schema::result_datasets::table
                    .filter(schema::result_datasets::task_id.eq(id))
                    .order(schema::result_datasets::id.asc())
                    .load(conn)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn fail(
        &self,
        scope: &AccessScope,
        id: i32,
        stage: &str,
        error_json: String,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let count = diesel::update(
                schema::simulation_tasks::table
                    .filter(schema::simulation_tasks::id.eq(id))
                    .filter(schema::simulation_tasks::session_id.eq_any(sessions)),
            )
            .set((
                schema::simulation_tasks::status.eq("failed"),
                schema::simulation_tasks::stage.eq(stage),
                schema::simulation_tasks::exit_code.eq::<Option<i32>>(None),
                schema::simulation_tasks::error_json.eq(error_json),
                schema::simulation_tasks::finished_at_ms.eq(now_ms),
            ))
            .execute(&mut conn)?;
            if count == 0 {
                Err(RepoError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, scope: &AccessScope, id: i32) -> RepoResult<SimulationTask> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(id))
                .filter(schema::simulation_tasks::session_id.eq_any(sessions))
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
