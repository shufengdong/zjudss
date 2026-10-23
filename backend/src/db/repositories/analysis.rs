use diesel::prelude::*;

use super::{RepoError, RepoResult, RunDataset};
use crate::auth::UserId;
use crate::db::models::{
    DatasetView, NewDatasetView, NewResultAnalysisRun, NewResultDataset, ResultAnalysisInput,
    ResultAnalysisRun,
};
use crate::db::{DbPool, schema};

#[derive(Clone)]
pub struct AnalysisRepo {
    pool: DbPool,
}

impl AnalysisRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        user: &UserId,
        run: NewResultAnalysisRun,
        mut inputs: Vec<ResultAnalysisInput>,
    ) -> RepoResult<ResultAnalysisRun> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                schema::sessions::table
                    .find(run.session_id)
                    .filter(schema::sessions::user_id.eq(&user))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                for input in &inputs {
                    owned_live_dataset(conn, &user, run.session_id, input.dataset_id)?;
                }
                diesel::insert_into(schema::result_analysis_runs::table)
                    .values(&run)
                    .execute(conn)?;
                let id = last_id(conn)?;
                for input in &mut inputs {
                    input.analysis_run_id = id;
                }
                diesel::insert_into(schema::result_analysis_inputs::table)
                    .values(&inputs)
                    .execute(conn)?;
                Ok(schema::result_analysis_runs::table.find(id).first(conn)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_stage(&self, user: &UserId, id: i32, stage: &str, now: i64) -> RepoResult<()> {
        self.update(user, id, "running", stage, None, Some(now), None)
            .await
    }

    pub async fn complete(
        &self,
        user: &UserId,
        id: i32,
        datasets: Vec<NewResultDataset>,
        now: i64,
    ) -> RepoResult<(ResultAnalysisRun, Vec<RunDataset>)> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                owned_analysis(conn, &user, id)?;
                for dataset in datasets {
                    diesel::insert_into(schema::result_datasets::table)
                        .values(&dataset)
                        .execute(conn)?;
                    let derived_id = last_id(conn)?;
                    diesel::sql_query(
                        "INSERT INTO dataset_lineage_snapshots(derived_dataset_id,source_dataset_id,source_name,source_schema_json,source_schema_hash,source_producer_json,created_at_ms) SELECT ?1,dataset_id,source_name,source_schema_json,source_schema_hash,source_producer_json,?2 FROM result_analysis_inputs WHERE analysis_run_id=?3",
                    )
                    .bind::<diesel::sql_types::Integer, _>(derived_id)
                    .bind::<diesel::sql_types::BigInt, _>(now)
                    .bind::<diesel::sql_types::Integer, _>(id)
                    .execute(conn)?;
                }
                diesel::update(schema::result_analysis_runs::table.find(id))
                    .set((
                        schema::result_analysis_runs::status.eq("succeeded"),
                        schema::result_analysis_runs::stage.eq("present"),
                        schema::result_analysis_runs::error_json.eq::<Option<String>>(None),
                        schema::result_analysis_runs::finished_at_ms.eq(Some(now)),
                    ))
                    .execute(conn)?;
                Ok((
                    schema::result_analysis_runs::table.find(id).first(conn)?,
                    diesel::sql_query("SELECT * FROM result_datasets WHERE analysis_run_id=?1 AND deleted_at_ms IS NULL ORDER BY id")
                        .bind::<diesel::sql_types::Integer,_>(id).load(conn)?,
                ))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn attach_view(
        &self,
        user: &UserId,
        source_dataset_id: i32,
        view: NewDatasetView,
    ) -> RepoResult<DatasetView> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                let session_id = dataset_session(conn, &user, source_dataset_id)?;
                if dataset_session(conn, &user, view.view_dataset_id)? != session_id
                    || view.source_dataset_id != source_dataset_id
                {
                    return Err(RepoError::NotFound);
                }
                diesel::insert_into(schema::dataset_views::table)
                    .values(&view)
                    .execute(conn)?;
                let id = last_id(conn)?;
                Ok(schema::dataset_views::table.find(id).first(conn)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn fail(
        &self,
        user: &UserId,
        id: i32,
        stage: &str,
        error: String,
        now: i64,
    ) -> RepoResult<()> {
        self.update(user, id, "failed", stage, Some(error), None, Some(now))
            .await
    }

    async fn update(
        &self,
        user: &UserId,
        id: i32,
        status: &str,
        stage: &str,
        error: Option<String>,
        started: Option<i64>,
        finished: Option<i64>,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let status = status.to_string();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let changed = diesel::sql_query("UPDATE result_analysis_runs SET status=?1,stage=?2,error_json=?3,started_at_ms=COALESCE(started_at_ms,?4),finished_at_ms=?5 WHERE id=?6 AND session_id IN(SELECT id FROM sessions WHERE user_id=?7)")
                .bind::<diesel::sql_types::Text,_>(status)
                .bind::<diesel::sql_types::Text,_>(stage)
                .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>,_>(error)
                .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>,_>(started)
                .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>,_>(finished)
                .bind::<diesel::sql_types::Integer,_>(id)
                .bind::<diesel::sql_types::Text,_>(user)
                .execute(&mut conn)?;
            if changed == 1 { Ok(()) } else { Err(RepoError::NotFound) }
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<ResultAnalysisRun> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_analysis(&mut conn, &user, id)
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn owned_analysis(
    conn: &mut SqliteConnection,
    user: &str,
    id: i32,
) -> RepoResult<ResultAnalysisRun> {
    schema::result_analysis_runs::table
        .inner_join(schema::sessions::table)
        .filter(schema::result_analysis_runs::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::result_analysis_runs::all_columns)
        .first(conn)
        .optional()?
        .ok_or(RepoError::NotFound)
}

fn owned_live_dataset(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
    dataset_id: i32,
) -> RepoResult<()> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        id: i32,
    }
    let found = diesel::sql_query(
        "SELECT d.id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.id=?2 AND s.user_id=?3 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(dataset_id)
    .bind::<diesel::sql_types::Integer, _>(session_id)
    .bind::<diesel::sql_types::Text, _>(user)
    .get_result::<Row>(conn)
    .optional()?;
    if found.is_some_and(|row| row.id == dataset_id) {
        Ok(())
    } else {
        Err(RepoError::NotFound)
    }
}

fn dataset_session(conn: &mut SqliteConnection, user: &str, id: i32) -> RepoResult<i32> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        session_id: i32,
    }
    diesel::sql_query(
        "SELECT s.id AS session_id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(id)
    .bind::<diesel::sql_types::Text, _>(user)
    .get_result::<Row>(conn)
    .optional()?
    .map(|row| row.session_id)
    .ok_or(RepoError::NotFound)
}

fn last_id(conn: &mut SqliteConnection) -> RepoResult<i32> {
    diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result(conn)
    .map_err(RepoError::from)
}
