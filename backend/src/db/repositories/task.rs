use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};

use crate::auth::AccessScope;
use crate::db::models::{NewSimulationTask, SimulationTask};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct TaskRepo {
    pool: DbPool,
}

#[derive(diesel::QueryableByName)]
struct LatestTaskRow {
    #[diesel(sql_type = Integer)]
    session_id: i32,
    #[diesel(sql_type = Text)]
    status: String,
}

impl TaskRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        scope: &AccessScope,
        new: NewSimulationTask,
    ) -> RepoResult<SimulationTask> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<SimulationTask> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction::<SimulationTask, diesel::result::Error, _>(|conn| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(new.session_id))
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)?;
                diesel::insert_into(schema::simulation_tasks::table)
                    .values(new)
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::simulation_tasks::table.find(id).first(conn)
            })
            .map_err(|error| match error {
                diesel::result::Error::NotFound => RepoError::NotFound,
                other => RepoError::Query(other),
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn mark_running(
        &self,
        scope: &AccessScope,
        task_id: i32,
        stage: &str,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let session_ids = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let target = schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(task_id))
                .filter(schema::simulation_tasks::session_id.eq_any(session_ids));
            let updated = diesel::update(target)
                .set((
                    schema::simulation_tasks::status.eq("running"),
                    schema::simulation_tasks::stage.eq(stage),
                    schema::simulation_tasks::started_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn mark_done(
        &self,
        scope: &AccessScope,
        task_id: i32,
        status: &str,
        stage: &str,
        exit_code: Option<i32>,
        error_json: Option<String>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let status = status.to_string();
        let stage = stage.to_string();
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let session_ids = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let target = schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(task_id))
                .filter(schema::simulation_tasks::session_id.eq_any(session_ids));
            let updated = diesel::update(target)
                .set((
                    schema::simulation_tasks::status.eq(status),
                    schema::simulation_tasks::stage.eq(stage),
                    schema::simulation_tasks::exit_code.eq(exit_code),
                    schema::simulation_tasks::error_json.eq(error_json),
                    schema::simulation_tasks::finished_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(
        &self,
        scope: &AccessScope,
        task_id: i32,
    ) -> RepoResult<Option<SimulationTask>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<SimulationTask>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let session_ids = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(task_id))
                .filter(schema::simulation_tasks::session_id.eq_any(session_ids))
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn latest_status_for_session(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> RepoResult<Option<String>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<String>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let exists = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()
                .map_err(RepoError::Query)?;
            if exists.is_none() {
                return Err(RepoError::NotFound);
            }
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::session_id.eq(session_id))
                .order(schema::simulation_tasks::id.desc())
                .select(schema::simulation_tasks::status)
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn latest_statuses(&self, scope: &AccessScope) -> RepoResult<HashMap<i32, String>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<HashMap<i32, String>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let rows: Vec<LatestTaskRow> = diesel::sql_query(
                "SELECT t.session_id, t.status FROM simulation_tasks t \
                 JOIN sessions s ON s.id = t.session_id \
                 JOIN (SELECT t2.session_id, MAX(t2.id) AS max_id \
                       FROM simulation_tasks t2 JOIN sessions s2 ON s2.id=t2.session_id \
                       WHERE s2.access_scope_kind=? AND s2.access_scope_key=? \
                       GROUP BY t2.session_id) m ON t.id = m.max_id \
                 WHERE s.access_scope_kind=? AND s.access_scope_key=?",
            )
            .bind::<Text, _>(&scope.kind)
            .bind::<Text, _>(&scope.key)
            .bind::<Text, _>(&scope.kind)
            .bind::<Text, _>(&scope.key)
            .load(&mut conn)
            .map_err(RepoError::Query)?;
            Ok(rows
                .into_iter()
                .map(|row| (row.session_id, row.status))
                .collect())
        })
        .await
        .map_err(RepoError::Join)?
    }
}
