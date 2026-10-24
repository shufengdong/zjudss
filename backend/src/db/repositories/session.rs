use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{NewSession, Session};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct SessionRepo {
    pool: DbPool,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, user_id: &UserId, title: &str, now_ms: i64) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Session, diesel::result::Error, _>(|conn| {
                diesel::insert_into(schema::sessions::table)
                    .values(NewSession {
                        title,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                        user_id: user_id.0.clone(),
                    })
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::sessions::table.find(id).first(conn)
            })
            .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(&self, user_id: &UserId) -> RepoResult<Vec<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::user_id.eq(user_id.0))
                .order(schema::sessions::updated_at_ms.desc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn touch(&self, user_id: &UserId, session_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let updated = diesel::update(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(user_id.0)),
            )
            .set(schema::sessions::updated_at_ms.eq(now_ms))
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

    pub async fn find(&self, user_id: &UserId, session_id: i32) -> RepoResult<Option<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        user_id: &UserId,
        session_id: i32,
        title: &str,
        now_ms: i64,
    ) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let target = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(&user_id.0));
            let updated = diesel::update(target)
                .set((
                    schema::sessions::title.eq(title),
                    schema::sessions::title_source.eq("user"),
                    schema::sessions::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(&self, user_id: &UserId, session_id: i32) -> RepoResult<Vec<i32>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<i32>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Vec<i32>, RepoError, _>(|conn| {
                let owns = schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(&user_id.0))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?;
                if owns.is_none() {
                    return Err(RepoError::NotFound);
                }

                #[derive(QueryableByName)]
                struct ArtifactId {
                    #[diesel(sql_type = diesel::sql_types::Integer)]
                    artifact_id: i32,
                }
                let artifact_ids = diesel::sql_query(
                    "SELECT artifact_id FROM (
                       SELECT d.artifact_id AS artifact_id
                       FROM draft_dataframes d
                       JOIN workspace_drafts wd ON wd.id=d.draft_id
                       JOIN workspaces w ON w.id=wd.workspace_id
                       WHERE w.session_id=?1
                       UNION ALL
                       SELECT d.artifact_id
                       FROM model_dataframes d
                       JOIN model_versions v ON v.id=d.version_id
                       JOIN workspaces w ON w.id=v.workspace_id
                       WHERE w.session_id=?1
                       UNION ALL
                       SELECT pending.artifact_id FROM (
                         SELECT DISTINCT i.id,c.artifact_id
                         FROM model_imports i
                         JOIN model_import_candidates c ON c.import_id=i.id
                         WHERE i.session_id=?1
                           AND i.status NOT IN ('applied','discarded','failed')
                           AND c.artifact_id IS NOT NULL
                       ) pending
                       UNION ALL
                       SELECT artifact_id FROM script_resources WHERE session_id=?1
                       UNION ALL
                       SELECT script_artifact_id FROM simulation_runs WHERE session_id=?1
                       UNION ALL
                       SELECT script_artifact_id FROM result_analysis_runs WHERE session_id=?1
                       UNION ALL
                       SELECT d.artifact_id
                       FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1 AND d.artifact_id IS NOT NULL
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .load::<ArtifactId>(conn)?
                .into_iter()
                .map(|row| row.artifact_id)
                .collect::<Vec<_>>();

                diesel::sql_query(
                    "UPDATE workspaces SET active_version_id=NULL WHERE session_id=?1",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM run_comparisons WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM dataset_views WHERE source_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM dataset_lineage_snapshots WHERE derived_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     ) OR source_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM result_analysis_inputs WHERE analysis_run_id IN (
                       SELECT id FROM result_analysis_runs WHERE session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM result_datasets WHERE run_id IN (
                       SELECT id FROM simulation_runs WHERE session_id=?1
                     ) OR analysis_run_id IN (
                       SELECT id FROM result_analysis_runs WHERE session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM messages WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM result_analysis_runs WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM simulation_runs WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM publish_receipts WHERE version_id IN (
                       SELECT v.id FROM model_versions v
                       JOIN workspaces w ON w.id=v.workspace_id
                       WHERE w.session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM script_resources WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM workspaces WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                let deleted = diesel::delete(
                    schema::sessions::table
                        .filter(schema::sessions::id.eq(session_id))
                        .filter(schema::sessions::user_id.eq(user_id.0)),
                )
                .execute(conn)?;
                debug_assert_eq!(deleted, 1);
                Ok(artifact_ids)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}
