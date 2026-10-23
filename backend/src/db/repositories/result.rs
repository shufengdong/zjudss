use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Integer, Nullable, Text};

use super::{RepoError, RepoResult, RunDataset};
use crate::auth::UserId;
use crate::db::DbPool;

#[derive(QueryableByName, Debug, Clone)]
pub struct ResultViewRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub source_dataset_id: i32,
    #[diesel(sql_type = Integer)]
    pub view_dataset_id: i32,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = Text)]
    pub spec_json: String,
    #[diesel(sql_type = Bool)]
    pub is_default: bool,
    #[diesel(sql_type = BigInt)]
    pub created_at_ms: i64,
}

#[derive(QueryableByName, Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageRow {
    #[diesel(sql_type = Integer)]
    pub source_dataset_id: i32,
    #[diesel(sql_type = Text)]
    pub source_name: String,
    #[diesel(sql_type = Text)]
    pub source_schema_json: String,
    #[diesel(sql_type = Text)]
    pub source_schema_hash: String,
    #[diesel(sql_type = Text)]
    pub source_producer_json: String,
    #[diesel(sql_type = Bool)]
    pub deleted: bool,
    #[diesel(sql_type = Nullable<Text>)]
    pub tombstone_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeletedResult {
    pub dataset_id: i32,
    pub session_id: i32,
    pub artifact_id: i32,
    pub tombstone_json: String,
}

#[derive(Debug)]
pub enum DeleteResultError {
    Repository(RepoError),
    Busy,
}
impl From<RepoError> for DeleteResultError {
    fn from(value: RepoError) -> Self {
        Self::Repository(value)
    }
}
impl From<diesel::result::Error> for DeleteResultError {
    fn from(value: diesel::result::Error) -> Self {
        Self::Repository(RepoError::Query(value))
    }
}

#[derive(Clone)]
pub struct ResultRepo {
    pool: DbPool,
}

impl ResultRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn find_live(&self, user: &UserId, id: i32) -> RepoResult<RunDataset> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            live_dataset_query()
                .bind::<Integer, _>(id)
                .bind::<Text, _>(user)
                .get_result::<RunDataset>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_analysis(
        &self,
        user: &UserId,
        analysis_id: i32,
    ) -> RepoResult<Vec<RunDataset>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let owned = diesel::sql_query(
                "SELECT count(*) AS count FROM result_analysis_runs a JOIN sessions s ON s.id=a.session_id WHERE a.id=?1 AND s.user_id=?2",
            )
            .bind::<Integer, _>(analysis_id)
            .bind::<Text, _>(&user)
            .get_result::<Count>(&mut conn)?
            .count;
            if owned == 0 {
                return Err(RepoError::NotFound);
            }
            diesel::sql_query("SELECT * FROM result_datasets WHERE analysis_run_id=?1 AND deleted_at_ms IS NULL AND artifact_id IS NOT NULL ORDER BY id")
                .bind::<Integer, _>(analysis_id)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_run(&self, user: &UserId, run_id: i32) -> RepoResult<Vec<RunDataset>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let owned = diesel::sql_query("SELECT count(*) AS count FROM simulation_runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=?1 AND s.user_id=?2 AND r.deleted_at_ms IS NULL")
                .bind::<Integer, _>(run_id)
                .bind::<Text, _>(&user)
                .get_result::<Count>(&mut conn)?
                .count;
            if owned == 0 {
                return Err(RepoError::NotFound);
            }
            diesel::sql_query("SELECT * FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL AND artifact_id IS NOT NULL ORDER BY id")
                .bind::<Integer, _>(run_id)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn views(&self, user: &UserId, dataset_id: i32) -> RepoResult<Vec<ResultViewRow>> {
        self.find_live(user, dataset_id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query("SELECT id,source_dataset_id,view_dataset_id,name,kind,spec_json,is_default,created_at_ms FROM dataset_views WHERE source_dataset_id=?1 ORDER BY id")
                .bind::<Integer, _>(dataset_id)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_view(
        &self,
        user: &UserId,
        view_id: i32,
    ) -> RepoResult<(ResultViewRow, RunDataset)> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let view = diesel::sql_query("SELECT v.id,v.source_dataset_id,v.view_dataset_id,v.name,v.kind,v.spec_json,v.is_default,v.created_at_ms FROM dataset_views v JOIN result_datasets d ON d.id=v.view_dataset_id LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE v.id=?1 AND s.user_id=?2 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL")
                .bind::<Integer, _>(view_id)
                .bind::<Text, _>(&user)
                .get_result::<ResultViewRow>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let dataset = diesel::sql_query("SELECT * FROM result_datasets WHERE id=?1 AND deleted_at_ms IS NULL AND artifact_id IS NOT NULL")
                .bind::<Integer, _>(view.view_dataset_id)
                .get_result::<RunDataset>(&mut conn)?;
            Ok((view, dataset))
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn lineage(&self, user: &UserId, id: i32) -> RepoResult<Vec<LineageRow>> {
        self.find_live(user, id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query("SELECT l.source_dataset_id,l.source_name,l.source_schema_json,l.source_schema_hash,l.source_producer_json,(d.deleted_at_ms IS NOT NULL) AS deleted,d.tombstone_json FROM dataset_lineage_snapshots l JOIN result_datasets d ON d.id=l.source_dataset_id WHERE l.derived_dataset_id=?1 ORDER BY l.id")
                .bind::<Integer, _>(id)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn session_id(&self, user: &UserId, id: i32) -> RepoResult<i32> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query("SELECT s.id AS value FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2")
                .bind::<Integer, _>(id)
                .bind::<Text, _>(user)
                .get_result::<IntValue>(&mut conn)
                .optional()?
                .map(|row| row.value)
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(
        &self,
        user: &UserId,
        id: i32,
        now: i64,
    ) -> Result<DeletedResult, DeleteResultError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<DeletedResult, DeleteResultError, _>(|conn| {
                let dataset = live_dataset_query()
                    .bind::<Integer, _>(id)
                    .bind::<Text, _>(&user)
                    .get_result::<RunDataset>(conn)
                    .optional()?
                    .ok_or(DeleteResultError::Repository(RepoError::NotFound))?;
                let busy = diesel::sql_query("SELECT count(*) AS count FROM result_analysis_inputs i JOIN result_analysis_runs a ON a.id=i.analysis_run_id WHERE i.dataset_id=?1 AND a.status IN ('pending','running')")
                    .bind::<Integer, _>(id)
                    .get_result::<Count>(conn)?
                    .count;
                if busy > 0 {
                    return Err(DeleteResultError::Busy);
                }
                let session_id = diesel::sql_query("SELECT s.id AS value FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1")
                    .bind::<Integer, _>(id)
                    .get_result::<IntValue>(conn)?
                    .value;
                let producer = if let Some(run_id) = dataset.run_id {
                    serde_json::json!({"kind":"run","runId":run_id.to_string()})
                } else {
                    serde_json::json!({"kind":"analysis","analysisRunId":dataset.analysis_run_id.map(|v|v.to_string())})
                };
                let tombstone = serde_json::json!({
                    "datasetId": id.to_string(),
                    "name": dataset.result_name,
                    "schema": serde_json::from_str::<serde_json::Value>(&dataset.schema_json).unwrap_or(serde_json::Value::Null),
                    "schemaHash": dataset.schema_hash,
                    "rowCount": dataset.row_count,
                    "sizeBytes": dataset.size_bytes,
                    "producer": producer,
                    "deletedAt": crate::application::ms_to_iso(now),
                })
                .to_string();
                let artifact_id = dataset.artifact_id.ok_or(DeleteResultError::Repository(RepoError::NotFound))?;
                let changed = diesel::sql_query("UPDATE result_datasets SET artifact_id=NULL,deleted_at_ms=?1,tombstone_json=?2 WHERE id=?3 AND deleted_at_ms IS NULL")
                    .bind::<BigInt, _>(now)
                    .bind::<Text, _>(&tombstone)
                    .bind::<Integer, _>(id)
                    .execute(conn)?;
                if changed != 1 {
                    return Err(DeleteResultError::Repository(RepoError::NotFound));
                }
                Ok(DeletedResult { dataset_id: id, session_id, artifact_id, tombstone_json: tombstone })
            })
        })
        .await
        .map_err(|error| DeleteResultError::Repository(RepoError::Join(error)))?
    }
}

fn live_dataset_query() -> diesel::query_builder::SqlQuery {
    diesel::sql_query(
        "SELECT d.* FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL",
    )
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct IntValue {
    #[diesel(sql_type = Integer)]
    value: i32,
}
