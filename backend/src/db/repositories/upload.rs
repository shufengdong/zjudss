use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{UploadBatchRecord, UploadCandidateRecord, UploadFileRecord};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct UploadBatchRecordSet {
    pub batch: UploadBatchRecord,
    pub files: Vec<UploadFileRecord>,
    pub candidates: Vec<UploadCandidateRecord>,
}

#[derive(Insertable)]
#[diesel(table_name = schema::upload_batches)]
struct NewBatch {
    session_id: i32,
    client_batch_id: String,
    source_kind: String,
    replacement_resource_id: Option<String>,
    status: String,
    model_publish_status: String,
    resource_publish_status: String,
    model_diagnostics_json: String,
    resource_diagnostics_json: String,
    parse_settings_json: String,
    diagnostics_json: String,
    reserved_bytes: i64,
    expires_at_ms: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Insertable)]
#[diesel(table_name = schema::upload_files)]
pub struct NewUploadFile {
    pub id: String,
    pub batch_id: i32,
    pub client_file_id: String,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub staged_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub source_artifact_id: Option<i32>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Clone)]
#[diesel(table_name = schema::upload_candidates)]
pub struct NewUploadCandidate {
    pub id: String,
    pub batch_id: i32,
    pub file_id: String,
    pub sheet_name: Option<String>,
    pub display_name: String,
    pub proposed_alias: Option<String>,
    pub normalized_alias: Option<String>,
    pub purpose: Option<String>,
    pub inferred_schema_ref: Option<String>,
    pub normalized_artifact_id: Option<i32>,
    pub schema_json: Option<String>,
    pub schema_hash: Option<String>,
    pub row_count: Option<i64>,
    pub evidence_json: String,
    pub selected: bool,
    pub created_at_ms: i64,
}

#[derive(Debug)]
pub struct SettledUpload {
    pub staged_paths: Vec<String>,
}

#[derive(Debug)]
pub struct RecoverableUpload {
    pub user_id: String,
    pub user_name: String,
    pub storage_scope_id: i32,
    pub batch_id: i32,
}

#[derive(Clone)]
pub struct UploadRepo {
    pool: DbPool,
}

impl UploadRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_reserved(
        &self,
        user: &UserId,
        scope_id: i32,
        session_id: i32,
        client_batch_id: String,
        source_kind: String,
        replacement_resource_id: Option<String>,
        reserved_bytes: i64,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> RepoResult<(UploadBatchRecord, bool)> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<(UploadBatchRecord, bool)> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let owned = schema::sessions::table
                    .inner_join(
                        schema::users::table.on(schema::users::id.eq(schema::sessions::user_id)),
                    )
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::users::storage_scope_id.eq(scope_id))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?;
                if owned.is_none() {
                    return Err(RepoError::NotFound);
                }
                if let Some(existing) = schema::upload_batches::table
                    .filter(schema::upload_batches::session_id.eq(session_id))
                    .filter(schema::upload_batches::client_batch_id.eq(&client_batch_id))
                    .first::<UploadBatchRecord>(conn)
                    .optional()?
                {
                    return Ok((existing, false));
                }
                let scope = schema::storage_scopes::table
                    .find(scope_id)
                    .first::<crate::db::models::StorageScope>(conn)?;
                if scope
                    .used_bytes
                    .saturating_add(scope.reserved_bytes)
                    .saturating_add(reserved_bytes)
                    > scope.byte_limit
                {
                    return Err(RepoError::QuotaExceeded);
                }
                diesel::update(schema::storage_scopes::table.find(scope_id))
                    .set((
                        schema::storage_scopes::reserved_bytes
                            .eq(scope.reserved_bytes + reserved_bytes),
                        schema::storage_scopes::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                diesel::insert_into(schema::upload_batches::table)
                    .values(NewBatch {
                        session_id,
                        client_batch_id,
                        source_kind,
                        replacement_resource_id,
                        status: "processing".into(),
                        model_publish_status: "not_applicable".into(),
                        resource_publish_status: "not_applicable".into(),
                        model_diagnostics_json: "[]".into(),
                        resource_diagnostics_json: "[]".into(),
                        parse_settings_json: "{}".into(),
                        diagnostics_json: "[]".into(),
                        reserved_bytes,
                        expires_at_ms,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                    })
                    .execute(conn)?;
                let batch = schema::upload_batches::table
                    .order(schema::upload_batches::id.desc())
                    .first(conn)
                    .map_err(RepoError::from)?;
                Ok((batch, true))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn add_files(&self, files: Vec<NewUploadFile>) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::insert_into(schema::upload_files::table)
                .values(files)
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<UploadBatchRecordSet> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<UploadBatchRecordSet> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let batch = schema::upload_batches::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                )
                .filter(schema::upload_batches::id.eq(id))
                .filter(schema::sessions::user_id.eq(user))
                .select(schema::upload_batches::all_columns)
                .first::<UploadBatchRecord>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let files = schema::upload_files::table
                .filter(schema::upload_files::batch_id.eq(id))
                .order(schema::upload_files::id.asc())
                .load(&mut conn)?;
            let candidates = schema::upload_candidates::table
                .filter(schema::upload_candidates::batch_id.eq(id))
                .order(schema::upload_candidates::id.asc())
                .load(&mut conn)?;
            Ok(UploadBatchRecordSet {
                batch,
                files,
                candidates,
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_by_client_id(
        &self,
        user: &UserId,
        session_id: i32,
        client_batch_id: &str,
    ) -> RepoResult<Option<UploadBatchRecordSet>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let client_batch_id = client_batch_id.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<UploadBatchRecordSet>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let id = schema::upload_batches::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                )
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::upload_batches::session_id.eq(session_id))
                .filter(schema::upload_batches::client_batch_id.eq(client_batch_id))
                .select(schema::upload_batches::id)
                .first::<i32>(&mut conn)
                .optional()?;
            let Some(id) = id else {
                return Ok(None);
            };
            let batch = schema::upload_batches::table.find(id).first(&mut conn)?;
            let files = schema::upload_files::table
                .filter(schema::upload_files::batch_id.eq(id))
                .order(schema::upload_files::id.asc())
                .load(&mut conn)?;
            let candidates = schema::upload_candidates::table
                .filter(schema::upload_candidates::batch_id.eq(id))
                .order(schema::upload_candidates::id.asc())
                .load(&mut conn)?;
            Ok(Some(UploadBatchRecordSet {
                batch,
                files,
                candidates,
            }))
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn replace_candidates(
        &self,
        batch_id: i32,
        candidates: Vec<NewUploadCandidate>,
        status: &str,
        parse_settings_json: String,
        diagnostics_json: String,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = schema::upload_batches::table
                    .find(batch_id)
                    .first::<UploadBatchRecord>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if matches!(
                    batch.status.as_str(),
                    "discarded" | "expired" | "confirmed" | "failed"
                ) {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                let has_model = candidates.iter().any(|candidate| {
                    candidate.selected && candidate.purpose.as_deref() == Some("model_table")
                });
                let has_resource = candidates.iter().any(|candidate| {
                    candidate.selected && candidate.purpose.as_deref() == Some("data_resource")
                });
                diesel::delete(
                    schema::upload_candidates::table
                        .filter(schema::upload_candidates::batch_id.eq(batch_id)),
                )
                .execute(conn)?;
                if !candidates.is_empty() {
                    diesel::insert_into(schema::upload_candidates::table)
                        .values(candidates)
                        .execute(conn)?;
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::status.eq(status),
                        schema::upload_batches::model_publish_status.eq(
                            if batch.model_publish_status == "published" {
                                "published"
                            } else if has_model {
                                "ready"
                            } else {
                                "not_applicable"
                            },
                        ),
                        schema::upload_batches::resource_publish_status.eq(
                            if batch.resource_publish_status == "published" {
                                "published"
                            } else if has_resource {
                                "ready"
                            } else {
                                "not_applicable"
                            },
                        ),
                        schema::upload_batches::parse_settings_json.eq(parse_settings_json),
                        schema::upload_batches::diagnostics_json.eq(diagnostics_json),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    #[allow(clippy::type_complexity)]
    pub async fn update_decisions(
        &self,
        batch_id: i32,
        decisions: Vec<(
            String,
            String,
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
        status: &str,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = schema::upload_batches::table
                    .find(batch_id)
                    .first::<UploadBatchRecord>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if matches!(
                    batch.status.as_str(),
                    "discarded" | "expired" | "confirmed" | "failed"
                ) {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                for (id, purpose, selected, display, alias, normalized) in decisions {
                    let target = schema::upload_candidates::table
                        .filter(schema::upload_candidates::batch_id.eq(batch_id))
                        .filter(schema::upload_candidates::id.eq(id));
                    let changed = if let Some(display) = display {
                        diesel::update(target)
                            .set((
                                schema::upload_candidates::purpose.eq(purpose),
                                schema::upload_candidates::selected.eq(selected),
                                schema::upload_candidates::display_name.eq(display),
                                schema::upload_candidates::proposed_alias.eq(alias),
                                schema::upload_candidates::normalized_alias.eq(normalized),
                            ))
                            .execute(conn)?
                    } else {
                        diesel::update(target)
                            .set((
                                schema::upload_candidates::purpose.eq(purpose),
                                schema::upload_candidates::selected.eq(selected),
                                schema::upload_candidates::proposed_alias.eq(alias),
                                schema::upload_candidates::normalized_alias.eq(normalized),
                            ))
                            .execute(conn)?
                    };
                    if changed == 0 {
                        return Err(RepoError::NotFound);
                    }
                }
                let purposes = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(schema::upload_candidates::selected.eq(true))
                    .select(schema::upload_candidates::purpose)
                    .load::<Option<String>>(conn)?;
                let has_model = purposes
                    .iter()
                    .any(|value| value.as_deref() == Some("model_table"));
                let has_resource = purposes
                    .iter()
                    .any(|value| value.as_deref() == Some("data_resource"));
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::status.eq(status),
                        schema::upload_batches::model_publish_status.eq(
                            if batch.model_publish_status == "published" {
                                "published"
                            } else if has_model {
                                "ready"
                            } else {
                                "not_applicable"
                            },
                        ),
                        schema::upload_batches::resource_publish_status.eq(
                            if batch.resource_publish_status == "published" {
                                "published"
                            } else if has_resource {
                                "ready"
                            } else {
                                "not_applicable"
                            },
                        ),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_controlled_mapping_schema(
        &self,
        user: &UserId,
        batch_id: i32,
        candidate_id: &str,
        schema_ref: &str,
        evidence_json: String,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let candidate_id = candidate_id.to_string();
        let schema_ref = schema_ref.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let status = schema::upload_batches::table
                    .inner_join(
                        schema::sessions::table
                            .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                    )
                    .filter(schema::upload_batches::id.eq(batch_id))
                    .filter(schema::sessions::user_id.eq(user))
                    .select(schema::upload_batches::status)
                    .first::<String>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if !matches!(status.as_str(), "awaiting_purpose" | "ready_to_confirm") {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                let changed = diesel::update(
                    schema::upload_candidates::table
                        .filter(schema::upload_candidates::batch_id.eq(batch_id))
                        .filter(schema::upload_candidates::id.eq(candidate_id)),
                )
                .set((
                    schema::upload_candidates::inferred_schema_ref.eq(Some(schema_ref)),
                    schema::upload_candidates::evidence_json.eq(evidence_json),
                ))
                .execute(conn)?;
                if changed != 1 {
                    return Err(RepoError::NotFound);
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set(schema::upload_batches::updated_at_ms.eq(now_ms))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn settle(
        &self,
        user: &UserId,
        batch_id: i32,
        status: &str,
        now_ms: i64,
    ) -> RepoResult<SettledUpload> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<SettledUpload> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let (batch, scope_id) = schema::upload_batches::table
                    .inner_join(
                        schema::sessions::table
                            .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                    )
                    .inner_join(
                        schema::users::table.on(schema::users::id.eq(schema::sessions::user_id)),
                    )
                    .filter(schema::upload_batches::id.eq(batch_id))
                    .filter(schema::sessions::user_id.eq(user))
                    .select((
                        schema::upload_batches::all_columns,
                        schema::users::storage_scope_id,
                    ))
                    .first::<(UploadBatchRecord, i32)>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if matches!(
                    batch.status.as_str(),
                    "discarded" | "expired" | "confirmed" | "failed"
                ) {
                    return Ok(SettledUpload {
                        staged_paths: Vec::new(),
                    });
                }
                let paths = schema::upload_files::table
                    .filter(schema::upload_files::batch_id.eq(batch_id))
                    .select(schema::upload_files::staged_path)
                    .load::<String>(conn)?;
                let artifacts = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(schema::upload_candidates::normalized_artifact_id.is_not_null())
                    .select(schema::upload_candidates::normalized_artifact_id)
                    .load::<Option<i32>>(conn)?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                let source_artifacts = if batch.resource_publish_status == "published" {
                    Vec::new()
                } else {
                    let ids = schema::upload_files::table
                        .filter(schema::upload_files::batch_id.eq(batch_id))
                        .filter(schema::upload_files::source_artifact_id.is_not_null())
                        .select(schema::upload_files::source_artifact_id)
                        .load::<Option<i32>>(conn)?
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>();
                    diesel::update(
                        schema::upload_files::table
                            .filter(schema::upload_files::batch_id.eq(batch_id)),
                    )
                    .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                    .execute(conn)?;
                    ids
                };
                let mut release_artifact_ids = artifacts;
                release_artifact_ids.extend(source_artifacts);
                super::artifact::release_refs(conn, scope_id, release_artifact_ids, now_ms)?;
                diesel::update(
                    schema::upload_candidates::table
                        .filter(schema::upload_candidates::batch_id.eq(batch_id)),
                )
                .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None))
                .execute(conn)?;
                diesel::update(schema::storage_scopes::table.find(scope_id))
                    .set((
                        schema::storage_scopes::reserved_bytes
                            .eq(schema::storage_scopes::reserved_bytes - batch.reserved_bytes),
                        schema::storage_scopes::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::status.eq(status),
                        schema::upload_batches::reserved_bytes.eq(0_i64),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(SettledUpload {
                    staged_paths: paths,
                })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn ids_expiring_before(&self, user: &UserId, before: i64) -> RepoResult<Vec<i32>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<i32>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::upload_batches::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                )
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::upload_batches::expires_at_ms.le(before))
                .filter(schema::upload_batches::status.ne_all([
                    "discarded",
                    "expired",
                    "confirmed",
                    "failed",
                ]))
                .select(schema::upload_batches::id)
                .load(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn recoverable(&self) -> RepoResult<Vec<RecoverableUpload>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<RecoverableUpload>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let rows = schema::upload_batches::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                )
                .inner_join(
                    schema::users::table.on(schema::users::id.eq(schema::sessions::user_id)),
                )
                .filter(schema::upload_batches::status.ne_all([
                    "discarded",
                    "expired",
                    "confirmed",
                    "failed",
                ]))
                .select((
                    schema::users::id,
                    schema::users::name,
                    schema::users::storage_scope_id,
                    schema::upload_batches::id,
                ))
                .load::<(String, String, i32, i32)>(&mut conn)?;
            Ok(rows
                .into_iter()
                .map(
                    |(user_id, user_name, storage_scope_id, batch_id)| RecoverableUpload {
                        user_id,
                        user_name,
                        storage_scope_id,
                        batch_id,
                    },
                )
                .collect())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn count_by_client_id(
        &self,
        user: &UserId,
        session_id: i32,
        client: &str,
    ) -> RepoResult<i64> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let client = client.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<i64> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::upload_batches::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                )
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::upload_batches::session_id.eq(session_id))
                .filter(schema::upload_batches::client_batch_id.eq(client))
                .count()
                .get_result(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn session_for_resource(&self, user: &UserId, resource_id: &str) -> RepoResult<i32> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let resource_id = resource_id.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<i32> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::data_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::data_resources::session_id)),
                )
                .filter(schema::data_resources::id.eq(resource_id))
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::data_resources::status.ne("deleted"))
                .select(schema::data_resources::session_id)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
