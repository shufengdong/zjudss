use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};

use crate::auth::UserId;
use crate::db::models::{ChartViewRow, NewChartView};
use crate::db::{DbPool, schema};

use super::artifact::{commit_draft_in_transaction, release_refs};
use super::{ArtifactDraft, RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct ChartViewDraft {
    pub source_dataset_id: i32,
    pub client_operation_id: String,
    pub request_hash: String,
    pub name: String,
    pub kind: String,
    pub config_json: String,
    pub config_hash: String,
    pub chart_contract_version: String,
    pub diagnostics_json: String,
    pub plot_schema_json: String,
    pub plot_schema_hash: String,
    pub plot_content_hash: String,
    pub row_count: i64,
    pub point_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
    pub artifact: ArtifactDraft,
}

#[derive(Debug, Clone)]
pub struct PublishedChartView {
    pub view: ChartViewRow,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct ChartViewRecord {
    pub view: ChartViewRow,
    pub source_deleted: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartViewSummaryRow {
    pub id: i32,
    pub session_id: i32,
    pub source_dataset_id: i32,
    pub source_name: String,
    pub source_deleted: bool,
    pub name: String,
    pub kind: String,
    pub point_count: i64,
    pub size_bytes: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DeletedChartView {
    pub view_id: i32,
    pub artifact_id: i32,
    pub tombstone_json: String,
}

#[derive(QueryableByName)]
struct SourceSnapshot {
    #[diesel(sql_type = Integer)]
    session_id: i32,
    #[diesel(sql_type = Integer)]
    storage_scope_id: i32,
    #[diesel(sql_type = Text)]
    source_name: String,
    #[diesel(sql_type = Text)]
    source_schema_json: String,
    #[diesel(sql_type = Text)]
    source_schema_hash: String,
    #[diesel(sql_type = Text)]
    source_content_hash: String,
}

#[derive(Clone)]
pub struct ChartViewRepo {
    pool: DbPool,
}

impl ChartViewRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn publish<F>(
        &self,
        user: &UserId,
        draft: ChartViewDraft,
        publish: F,
    ) -> RepoResult<PublishedChartView>
    where
        F: FnOnce(&str) -> Result<(), RepoError> + Send + 'static,
    {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let mut publish = Some(publish);
            conn.immediate_transaction::<PublishedChartView, RepoError, _>(|conn| {
                let replay = schema::dataset_views::table
                    .inner_join(schema::sessions::table)
                    .filter(schema::dataset_views::source_dataset_id.eq(draft.source_dataset_id))
                    .filter(
                        schema::dataset_views::client_operation_id.eq(&draft.client_operation_id),
                    )
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::dataset_views::all_columns)
                    .first::<ChartViewRow>(conn)
                    .optional()?;
                if let Some(view) = replay {
                    if view.deleted_at_ms.is_some() {
                        return Err(RepoError::InvalidState("chart_operation_gone".into()));
                    }
                    if view.request_hash != draft.request_hash {
                        return Err(RepoError::IdempotencyConflict);
                    }
                    return Ok(PublishedChartView {
                        view,
                        replayed: true,
                    });
                }
                let source = live_source(conn, &user, draft.source_dataset_id)?;
                let name_taken = schema::dataset_views::table
                    .filter(schema::dataset_views::source_dataset_id.eq(draft.source_dataset_id))
                    .filter(schema::dataset_views::name.eq(&draft.name))
                    .filter(schema::dataset_views::deleted_at_ms.is_null())
                    .select(schema::dataset_views::id)
                    .first::<i32>(conn)
                    .optional()?
                    .is_some();
                if name_taken {
                    return Err(RepoError::InvalidState("chart_name_conflict".into()));
                }
                if draft.plot_content_hash != draft.artifact.sha256
                    || draft.size_bytes != draft.artifact.size_bytes
                {
                    return Err(RepoError::InvalidState(
                        "chart_artifact_metadata_mismatch".into(),
                    ));
                }
                let artifact = commit_draft_in_transaction(
                    conn,
                    source.storage_scope_id,
                    &draft.artifact,
                    publish.take().expect("publish once"),
                )?;
                let row = NewChartView {
                    session_id: source.session_id,
                    source_dataset_id: draft.source_dataset_id,
                    artifact_id: Some(artifact.id),
                    client_operation_id: draft.client_operation_id.clone(),
                    request_hash: draft.request_hash.clone(),
                    name: draft.name.clone(),
                    kind: draft.kind.clone(),
                    config_json: draft.config_json.clone(),
                    config_hash: draft.config_hash.clone(),
                    chart_contract_version: draft.chart_contract_version.clone(),
                    diagnostics_json: draft.diagnostics_json.clone(),
                    plot_schema_json: draft.plot_schema_json.clone(),
                    plot_schema_hash: draft.plot_schema_hash.clone(),
                    plot_content_hash: draft.plot_content_hash.clone(),
                    row_count: draft.row_count,
                    point_count: draft.point_count,
                    size_bytes: draft.size_bytes,
                    source_name: source.source_name,
                    source_schema_json: source.source_schema_json,
                    source_schema_hash: source.source_schema_hash,
                    source_content_hash: source.source_content_hash,
                    created_at_ms: draft.created_at_ms,
                    deleted_at_ms: None,
                    tombstone_json: None,
                };
                diesel::insert_into(schema::dataset_views::table)
                    .values(&row)
                    .execute(conn)?;
                let view = schema::dataset_views::table
                    .order(schema::dataset_views::id.desc())
                    .first(conn)?;
                Ok(PublishedChartView {
                    view,
                    replayed: false,
                })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_operation(
        &self,
        user: &UserId,
        source_dataset_id: i32,
        client_operation_id: &str,
    ) -> RepoResult<Option<ChartViewRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let client_operation_id = client_operation_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::dataset_views::table
                .inner_join(schema::sessions::table)
                .filter(schema::dataset_views::source_dataset_id.eq(source_dataset_id))
                .filter(schema::dataset_views::client_operation_id.eq(client_operation_id))
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::dataset_views::all_columns)
                .first::<ChartViewRow>(&mut conn)
                .optional()
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<ChartViewRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let view = schema::dataset_views::table
                .inner_join(schema::sessions::table)
                .filter(schema::dataset_views::id.eq(id))
                .filter(schema::dataset_views::deleted_at_ms.is_null())
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::dataset_views::all_columns)
                .first::<ChartViewRow>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let source_deleted = schema::result_datasets::table
                .find(view.source_dataset_id)
                .select(schema::result_datasets::deleted_at_ms)
                .first::<Option<i64>>(&mut conn)?
                .is_some();
            Ok(ChartViewRecord {
                view,
                source_deleted,
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(
        &self,
        user: &UserId,
        session_id: i32,
        before: Option<(i64, i32)>,
        limit: i64,
    ) -> RepoResult<Vec<ChartViewSummaryRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let owned = schema::sessions::table
                .find(session_id)
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()?
                .is_some();
            if !owned {
                return Err(RepoError::NotFound);
            }
            let mut query = schema::dataset_views::table
                .filter(schema::dataset_views::session_id.eq(session_id))
                .filter(schema::dataset_views::deleted_at_ms.is_null())
                .into_boxed();
            if let Some((created_at, id)) = before {
                query = query.filter(
                    schema::dataset_views::created_at_ms.lt(created_at).or(
                        schema::dataset_views::created_at_ms
                            .eq(created_at)
                            .and(schema::dataset_views::id.lt(id)),
                    ),
                );
            }
            let views = query
                .order((
                    schema::dataset_views::created_at_ms.desc(),
                    schema::dataset_views::id.desc(),
                ))
                .limit(limit)
                .load::<ChartViewRow>(&mut conn)?;
            views
                .into_iter()
                .map(|view| {
                    let source_deleted = schema::result_datasets::table
                        .find(view.source_dataset_id)
                        .select(schema::result_datasets::deleted_at_ms)
                        .first::<Option<i64>>(&mut conn)?
                        .is_some();
                    Ok(ChartViewSummaryRow {
                        id: view.id,
                        session_id: view.session_id,
                        source_dataset_id: view.source_dataset_id,
                        source_name: view.source_name,
                        source_deleted,
                        name: view.name,
                        kind: view.kind,
                        point_count: view.point_count,
                        size_bytes: view.size_bytes,
                        created_at_ms: view.created_at_ms,
                    })
                })
                .collect()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(
        &self,
        user: &UserId,
        id: i32,
        now_ms: i64,
    ) -> RepoResult<DeletedChartView> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<DeletedChartView, RepoError, _>(|conn| {
                let (view, scope_id) = schema::dataset_views::table
                    .inner_join(schema::sessions::table.inner_join(schema::users::table))
                    .filter(schema::dataset_views::id.eq(id))
                    .filter(schema::dataset_views::deleted_at_ms.is_null())
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select((
                        schema::dataset_views::all_columns,
                        schema::users::storage_scope_id,
                    ))
                    .first::<(ChartViewRow, i32)>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                let artifact_id = view
                    .artifact_id
                    .ok_or_else(|| RepoError::InvalidState("chart artifact missing".into()))?;
                let tombstone_json = serde_json::json!({
                    "name": view.name,
                    "kind": view.kind,
                    "sourceDatasetId": view.source_dataset_id.to_string(),
                    "sourceName": view.source_name,
                    "configHash": view.config_hash,
                    "plotContentHash": view.plot_content_hash,
                    "deletedAt": now_ms,
                })
                .to_string();
                diesel::update(schema::dataset_views::table.find(id))
                    .set((
                        schema::dataset_views::artifact_id.eq(Option::<i32>::None),
                        schema::dataset_views::deleted_at_ms.eq(Some(now_ms)),
                        schema::dataset_views::tombstone_json.eq(Some(&tombstone_json)),
                    ))
                    .execute(conn)?;
                release_refs(conn, scope_id, [artifact_id], now_ms)?;
                Ok(DeletedChartView {
                    view_id: id,
                    artifact_id,
                    tombstone_json,
                })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn live_source(
    conn: &mut SqliteConnection,
    user: &str,
    dataset_id: i32,
) -> RepoResult<SourceSnapshot> {
    diesel::sql_query(
        "SELECT s.id session_id,u.storage_scope_id,d.result_name source_name,
                d.schema_json source_schema_json,d.schema_hash source_schema_hash,
                ar.sha256 source_content_hash
         FROM result_datasets d
         JOIN artifacts ar ON ar.id=d.artifact_id
         LEFT JOIN simulation_runs r ON r.id=d.run_id
         LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
         JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id)
         JOIN users u ON u.id=s.user_id
         WHERE d.id=?1 AND s.user_id=?2 AND s.deletion_status='live'
           AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL",
    )
    .bind::<Integer, _>(dataset_id)
    .bind::<Text, _>(user)
    .get_result::<SourceSnapshot>(conn)
    .optional()?
    .ok_or(RepoError::NotFound)
}
