use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use crate::auth::UserId;
use crate::db::models::{DataResourceRecord, UploadBatchRecord};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[path = "data_resource_deletion.rs"]
mod data_resource_deletion;
#[path = "data_resource_replacement.rs"]
mod data_resource_replacement;

pub use self::data_resource_deletion::{DeletedResource, DeletionImpactRow, DeletionPreviewInput};
pub use self::data_resource_replacement::{StructuredPreviewConfirmation, StructuredPreviewInput};

#[derive(Debug, Clone)]
pub struct ResourcePageRow {
    pub resource: DataResourceRecord,
    pub normalized_sha256: String,
    pub normalized_size_bytes: i64,
    pub source_sha256: Option<String>,
    pub source_size_bytes: Option<i64>,
}

pub struct PromoteDatasetInput {
    pub session_id: i32,
    pub dataset_id: i32,
    pub resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub normalized_alias: String,
    pub now_ms: i64,
}

#[derive(Clone)]
pub struct DataResourceRepo {
    pub(super) pool: DbPool,
}

impl DataResourceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn list(
        &self,
        user: &UserId,
        session_id: i32,
        before: Option<(i64, String)>,
        limit: i64,
    ) -> RepoResult<Vec<ResourcePageRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &user, session_id)?;
            let mut query = schema::data_resources::table
                .filter(schema::data_resources::session_id.eq(session_id))
                .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                .into_boxed();
            if let Some((created, id)) = before {
                query = query.filter(
                    schema::data_resources::created_at_ms.lt(created).or(
                        schema::data_resources::created_at_ms
                            .eq(created)
                            .and(schema::data_resources::id.lt(id)),
                    ),
                );
            }
            let rows = query
                .order((
                    schema::data_resources::created_at_ms.desc(),
                    schema::data_resources::id.desc(),
                ))
                .limit(limit)
                .load::<DataResourceRecord>(&mut conn)?;
            rows.into_iter()
                .map(|resource| enrich_resource(&mut conn, resource))
                .collect()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_current_for_user(
        &self,
        user: &UserId,
    ) -> RepoResult<Vec<DataResourceRecord>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::data_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::data_resources::session_id)),
                )
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                .select(schema::data_resources::all_columns)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn compatible_for_session(
        &self,
        user: &UserId,
        session_id: i32,
        schema_ref: &str,
    ) -> RepoResult<Vec<DataResourceRecord>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let schema_ref = schema_ref.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &user, session_id)?;
            schema::data_resources::table
                .filter(schema::data_resources::session_id.eq(session_id))
                .filter(schema::data_resources::status.eq("live"))
                .filter(schema::data_resources::declared_schema_ref.eq(Some(schema_ref)))
                .order((
                    schema::data_resources::created_at_ms.desc(),
                    schema::data_resources::id.desc(),
                ))
                .load(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(
        &self,
        user: &UserId,
        id: &str,
        include_historical: bool,
    ) -> RepoResult<ResourcePageRow> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let resource = owned_resource(&mut conn, &user, &id, include_historical)?;
            enrich_resource(&mut conn, resource)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_record(&self, user: &UserId, id: &str) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_resource(&mut conn, &user, &id, true)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn for_batch(
        &self,
        user: &UserId,
        batch_id: i32,
    ) -> RepoResult<Vec<ResourcePageRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_batch(&mut conn, &user, batch_id)?;
            let rows = schema::data_resources::table
                .filter(schema::data_resources::source_batch_id.eq(batch_id))
                .filter(schema::data_resources::status.ne("deleted"))
                .order(schema::data_resources::created_at_ms.asc())
                .load::<DataResourceRecord>(&mut conn)?;
            rows.into_iter()
                .map(|row| enrich_resource(&mut conn, row))
                .collect()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn mark_needs_rename(&self, user: &UserId, id: &str, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_resource(&mut conn, &user, &id, false)?;
            diesel::update(schema::data_resources::table.find(&id))
                .filter(schema::data_resources::status.eq("live"))
                .set((
                    schema::data_resources::status.eq("needs_rename"),
                    schema::data_resources::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        user: &UserId,
        id: &str,
        display_name: Option<String>,
        alias: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let current = owned_resource(&mut conn, &user, &id, false)?;
            if let Some((_, normalized)) = &alias {
                let conflict = schema::data_resources::table
                    .filter(schema::data_resources::session_id.eq(current.session_id))
                    .filter(schema::data_resources::normalized_alias.eq(normalized))
                    .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                    .filter(schema::data_resources::id.ne(&id))
                    .select(schema::data_resources::id)
                    .first::<String>(&mut conn)
                    .optional()?
                    .is_some();
                if conflict {
                    return Err(RepoError::InvalidState("resource_alias_conflict".into()));
                }
            }
            diesel::update(schema::data_resources::table.find(&id))
                .set((
                    schema::data_resources::display_name
                        .eq(display_name.unwrap_or(current.display_name)),
                    schema::data_resources::alias
                        .eq(alias.as_ref().map(|v| v.0.clone()).unwrap_or(current.alias)),
                    schema::data_resources::normalized_alias
                        .eq(alias.map(|v| v.1).unwrap_or(current.normalized_alias)),
                    schema::data_resources::status.eq("live"),
                    schema::data_resources::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            schema::data_resources::table
                .find(id)
                .first(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn promote_dataset(
        &self,
        user: &UserId,
        input: PromoteDatasetInput,
    ) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let PromoteDatasetInput {
            session_id,
            dataset_id,
            resource_id,
            display_name,
            alias,
            normalized_alias,
            now_ms,
        } = input;
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                ensure_session(conn, &user, session_id)?;
                #[derive(QueryableByName)]
                struct Dataset {
                    #[diesel(sql_type = Integer)]
                    artifact_id: i32,
                    #[diesel(sql_type = Text)]
                    result_name: String,
                    #[diesel(sql_type = Text)]
                    schema_json: String,
                    #[diesel(sql_type = Text)]
                    schema_hash: String,
                    #[diesel(sql_type = BigInt)]
                    row_count: i64,
                    #[diesel(sql_type = diesel::sql_types::Nullable<Integer>)]
                    run_id: Option<i32>,
                    #[diesel(sql_type = diesel::sql_types::Nullable<Integer>)]
                    analysis_run_id: Option<i32>,
                    #[diesel(sql_type = Integer)]
                    source_session_id: i32,
                }
                let dataset = diesel::sql_query("SELECT d.artifact_id,d.result_name,d.schema_json,d.schema_hash,d.row_count,d.run_id,d.analysis_run_id,COALESCE(r.session_id,a.session_id) source_session_id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL")
                    .bind::<Integer, _>(dataset_id)
                    .bind::<Text, _>(&user)
                    .get_result::<Dataset>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if dataset.source_session_id != session_id {
                    return Err(RepoError::NotFound);
                }
                let mut final_alias = alias;
                let mut final_normalized = normalized_alias;
                let conflict = schema::data_resources::table
                    .filter(schema::data_resources::session_id.eq(session_id))
                    .filter(schema::data_resources::normalized_alias.eq(&final_normalized))
                    .filter(schema::data_resources::status.ne("deleted"))
                    .select(schema::data_resources::id)
                    .first::<String>(conn)
                    .optional()?
                    .is_some();
                if conflict {
                    final_alias = format!(
                        "data_{}",
                        &resource_id[resource_id.len().saturating_sub(8)..]
                    );
                    final_normalized = final_alias.clone();
                }
                let source_kind = if dataset.analysis_run_id.is_some() {
                    "analysis"
                } else {
                    "result"
                };
                let record = DataResourceRecord {
                    id: resource_id,
                    session_id,
                    display_name,
                    alias: final_alias,
                    normalized_alias: final_normalized,
                    status: "live".into(),
                    declared_schema_ref: None,
                    source_kind: source_kind.into(),
                    normalized_artifact_id: Some(dataset.artifact_id),
                    source_artifact_id: None,
                    schema_json: dataset.schema_json,
                    schema_hash: dataset.schema_hash,
                    row_count: dataset.row_count,
                    source_batch_id: None,
                    source_model_version_id: None,
                    replaced_by_resource_id: None,
                    lineage_json: serde_json::json!({
                        "datasetId": dataset_id,
                        "runId": dataset.run_id,
                        "analysisRunId": dataset.analysis_run_id,
                        "resultName": dataset.result_name,
                    })
                    .to_string(),
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    deleted_at_ms: None,
                    tombstone_json: None,
                };
                diesel::insert_into(schema::data_resources::table)
                    .values(&record)
                    .execute(conn)?;
                let scope_id = super::artifact_refs::storage_scope_id_for_user(
                    conn,
                    &UserId(user.clone()),
                )?;
                super::artifact_refs::retain_ref_count(conn, scope_id, dataset.artifact_id, 1)?;
                Ok(record)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}

pub(super) fn ensure_session(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
) -> RepoResult<()> {
    let exists = schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(RepoError::NotFound)
    }
}
pub(super) fn owned_batch(
    conn: &mut SqliteConnection,
    user: &str,
    id: i32,
) -> RepoResult<UploadBatchRecord> {
    schema::upload_batches::table
        .inner_join(
            schema::sessions::table.on(schema::sessions::id.eq(schema::upload_batches::session_id)),
        )
        .filter(schema::upload_batches::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::upload_batches::all_columns)
        .first(conn)
        .optional()?
        .ok_or(RepoError::NotFound)
}
fn owned_resource(
    conn: &mut SqliteConnection,
    user: &str,
    id: &str,
    historical: bool,
) -> RepoResult<DataResourceRecord> {
    let mut query = schema::data_resources::table
        .inner_join(
            schema::sessions::table.on(schema::sessions::id.eq(schema::data_resources::session_id)),
        )
        .filter(schema::data_resources::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::data_resources::all_columns)
        .into_boxed();
    if !historical {
        query = query.filter(schema::data_resources::status.eq_any(["live", "needs_rename"]));
    }
    query.first(conn).optional()?.ok_or(RepoError::NotFound)
}
fn enrich_resource(
    conn: &mut SqliteConnection,
    resource: DataResourceRecord,
) -> RepoResult<ResourcePageRow> {
    let normalized_artifact_id = resource
        .normalized_artifact_id
        .ok_or_else(|| RepoError::InvalidState("资源 artifact 已释放".into()))?;
    let normalized = schema::artifacts::table
        .find(normalized_artifact_id)
        .select((schema::artifacts::sha256, schema::artifacts::size_bytes))
        .first::<(String, i64)>(conn)?;
    let source = resource
        .source_artifact_id
        .map(|id| {
            schema::artifacts::table
                .find(id)
                .select((schema::artifacts::sha256, schema::artifacts::size_bytes))
                .first::<(String, i64)>(conn)
        })
        .transpose()?;
    Ok(ResourcePageRow {
        resource,
        normalized_sha256: normalized.0,
        normalized_size_bytes: normalized.1,
        source_sha256: source.as_ref().map(|v| v.0.clone()),
        source_size_bytes: source.map(|v| v.1),
    })
}
