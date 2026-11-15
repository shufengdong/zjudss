use diesel::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};

use diesel::sql_types::{BigInt, Bool, Integer, Nullable, Text};

use super::{RepoError, RepoResult, RunDataset};
use crate::auth::UserId;
use crate::db::DbPool;

#[derive(QueryableByName, Debug, Clone)]
pub struct ResultViewRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = BigInt)]
    pub created_at_ms: i64,
}

#[derive(QueryableByName, Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageRow {
    #[diesel(sql_type = Text)]
    pub source_kind: String,
    #[diesel(sql_type = Nullable<Integer>)]
    pub source_dataset_id: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    pub source_resource_id: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub model_version_id: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    pub model_dataframe_id: Option<i32>,
    #[diesel(sql_type = Text)]
    pub source_name: String,
    #[diesel(sql_type = Text)]
    pub source_schema_json: String,
    #[diesel(sql_type = Text)]
    pub source_schema_hash: String,
    #[diesel(sql_type = Text)]
    pub source_content_hash: String,
    #[diesel(sql_type = Text)]
    pub source_producer_json: String,
    #[diesel(sql_type = Bool)]
    pub deleted: bool,
    #[diesel(sql_type = Nullable<Text>)]
    pub tombstone_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetModelOrigin {
    Unique(i32),
    None,
    Multiple(Vec<i32>),
    Invalid,
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
                "SELECT count(*) AS count FROM result_analysis_runs a JOIN sessions s ON s.id=a.session_id WHERE a.id=?1 AND s.user_id=?2 AND s.deletion_status='live'",
            )
            .bind::<Integer, _>(analysis_id)
            .bind::<Text, _>(&user)
            .get_result::<Count>(&mut conn)?
            .count;
            if owned == 0 {
                return Err(RepoError::NotFound);
            }
            diesel::sql_query("SELECT d.* FROM result_datasets d WHERE d.analysis_run_id=?1 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL ORDER BY d.id")
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
            let owned = diesel::sql_query("SELECT count(*) AS count FROM simulation_runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=?1 AND s.user_id=?2 AND s.deletion_status='live' AND r.deleted_at_ms IS NULL")
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
            diesel::sql_query("SELECT id,name,kind,created_at_ms FROM dataset_views WHERE source_dataset_id=?1 AND deleted_at_ms IS NULL ORDER BY id")
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
            let view = diesel::sql_query("SELECT v.id,v.name,v.kind,v.created_at_ms FROM dataset_views v JOIN sessions s ON s.id=v.session_id WHERE v.id=?1 AND s.user_id=?2 AND s.deletion_status='live' AND v.deleted_at_ms IS NULL")
                .bind::<Integer, _>(view_id)
                .bind::<Text, _>(&user)
                .get_result::<ResultViewRow>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let dataset = diesel::sql_query("SELECT d.* FROM result_datasets d JOIN dataset_views v ON v.source_dataset_id=d.id WHERE v.id=?1")
                .bind::<Integer, _>(view.id)
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
            diesel::sql_query("SELECT l.source_kind,l.source_dataset_id,l.source_resource_id,l.model_version_id,l.model_dataframe_id,l.source_name,l.source_schema_json,l.source_schema_hash,l.source_content_hash,l.source_producer_json,CASE WHEN l.source_kind='dataset' THEN COALESCE(d.deleted_at_ms IS NOT NULL,1) WHEN l.source_kind='data_resource' THEN COALESCE(r.status='deleted',1) ELSE (v.id IS NULL OR v.deleted_at_ms IS NOT NULL) END AS deleted,CASE WHEN l.source_kind='dataset' THEN d.tombstone_json WHEN l.source_kind='data_resource' THEN r.tombstone_json ELSE v.tombstone_json END tombstone_json FROM dataset_lineage_snapshots l LEFT JOIN result_datasets d ON d.id=l.source_dataset_id LEFT JOIN data_resources r ON r.id=l.source_resource_id LEFT JOIN model_versions v ON v.id=l.model_version_id WHERE l.derived_dataset_id=?1 ORDER BY l.id")
                .bind::<Integer, _>(id)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn dataset_model_origin(
        &self,
        user: &UserId,
        session_id: i32,
        dataset_id: i32,
        max_depth: usize,
    ) -> RepoResult<DatasetModelOrigin> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let owned = diesel::sql_query("SELECT count(*) count FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.id=?2 AND s.user_id=?3 AND s.deletion_status='live' AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL")
                .bind::<Integer, _>(dataset_id)
                .bind::<Integer, _>(session_id)
                .bind::<Text, _>(&user)
                .get_result::<Count>(&mut conn)?
                .count;
            if owned != 1 {
                return Err(RepoError::NotFound);
            }
            let mut visiting = HashSet::new();
            let mut memo = HashMap::new();
            let Some(summary) = collect_model_origins(
                &mut conn,
                session_id,
                dataset_id,
                0,
                max_depth,
                &mut visiting,
                &mut memo,
            )?
            else {
                return Ok(DatasetModelOrigin::Invalid);
            };
            Ok(match summary.versions.len() {
                0 => DatasetModelOrigin::None,
                1 => DatasetModelOrigin::Unique(*summary.versions.first().expect("one origin")),
                _ => DatasetModelOrigin::Multiple(summary.versions.into_iter().collect()),
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn session_id(&self, user: &UserId, id: i32) -> RepoResult<i32> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query("SELECT s.id AS value FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND s.deletion_status='live'")
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

#[derive(QueryableByName)]
struct OriginDatasetRow {
    #[diesel(sql_type = Nullable<Integer>)]
    run_id: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    model_version_id: Option<i32>,
}

#[derive(QueryableByName)]
struct OriginLineageRow {
    #[diesel(sql_type = Text)]
    source_kind: String,
    #[diesel(sql_type = Nullable<Integer>)]
    source_dataset_id: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    model_version_id: Option<i32>,
}

#[derive(Clone)]
struct OriginSummary {
    versions: BTreeSet<i32>,
    max_descendant_depth: usize,
}

#[allow(clippy::too_many_arguments)]
fn collect_model_origins(
    conn: &mut SqliteConnection,
    session_id: i32,
    dataset_id: i32,
    depth: usize,
    max_depth: usize,
    visiting: &mut HashSet<i32>,
    memo: &mut HashMap<i32, OriginSummary>,
) -> RepoResult<Option<OriginSummary>> {
    if depth > max_depth {
        return Ok(None);
    }
    if let Some(summary) = memo.get(&dataset_id) {
        return Ok((depth + summary.max_descendant_depth <= max_depth).then(|| summary.clone()));
    }
    if !visiting.insert(dataset_id) {
        return Ok(None);
    }
    let dataset = diesel::sql_query("SELECT d.run_id,r.model_version_id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id WHERE d.id=?1 AND COALESCE(r.session_id,a.session_id)=?2")
        .bind::<Integer, _>(dataset_id)
        .bind::<Integer, _>(session_id)
        .get_result::<OriginDatasetRow>(conn)
        .optional()?;
    let Some(dataset) = dataset else {
        visiting.remove(&dataset_id);
        return Ok(None);
    };
    let summary = if dataset.run_id.is_some() {
        let mut versions = BTreeSet::new();
        if let Some(version_id) = dataset.model_version_id {
            versions.insert(version_id);
        }
        Some(OriginSummary {
            versions,
            max_descendant_depth: 0,
        })
    } else {
        let sources = diesel::sql_query("SELECT source_kind,source_dataset_id,model_version_id FROM dataset_lineage_snapshots WHERE derived_dataset_id=?1 ORDER BY id")
            .bind::<Integer, _>(dataset_id)
            .load::<OriginLineageRow>(conn)?;
        let mut summary = OriginSummary {
            versions: BTreeSet::new(),
            max_descendant_depth: 0,
        };
        let mut valid = true;
        for source in sources {
            match source.source_kind.as_str() {
                "dataset" => {
                    let Some(source_id) = source.source_dataset_id else {
                        valid = false;
                        break;
                    };
                    let Some(child) = collect_model_origins(
                        conn,
                        session_id,
                        source_id,
                        depth + 1,
                        max_depth,
                        visiting,
                        memo,
                    )?
                    else {
                        valid = false;
                        break;
                    };
                    summary.versions.extend(child.versions);
                    summary.max_descendant_depth = summary
                        .max_descendant_depth
                        .max(child.max_descendant_depth + 1);
                }
                "active_model" => {
                    let Some(version_id) = source.model_version_id else {
                        valid = false;
                        break;
                    };
                    summary.versions.insert(version_id);
                }
                "data_resource" => {}
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        (valid && depth + summary.max_descendant_depth <= max_depth).then_some(summary)
    };
    visiting.remove(&dataset_id);
    if let Some(summary) = &summary {
        memo.insert(dataset_id, summary.clone());
    }
    Ok(summary)
}

fn live_dataset_query() -> diesel::query_builder::SqlQuery {
    diesel::sql_query(
        "SELECT d.* FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND s.deletion_status='live' AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL",
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
