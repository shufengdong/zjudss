use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer};

use crate::application::upload_candidate_policy::settled_status;
use crate::auth::UserId;
use crate::db::models::{DataResourceRecord, DraftRef, UploadCandidateRecord, UploadFileRecord};
use crate::db::{DbPool, schema};

use super::{DataResourceRepo, RepoResult};
use super::{RepoError, data_resource};

#[path = "upload_confirmation_transactions.rs"]
pub(super) mod transactions;

#[derive(Debug, Clone)]
pub struct ResourcePublishInput {
    pub candidate_id: String,
    pub resource_id: String,
    pub alias: String,
    pub normalized_alias: String,
    pub display_name: String,
    pub source_kind: String,
    pub lineage_json: String,
    pub alias_explicit: bool,
}

#[derive(Debug, Clone)]
pub struct PublishChildResult {
    pub resources: Vec<DataResourceRecord>,
    pub replaced_artifact_ids: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct FinishedUploadBatch {
    pub staged_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftApplicationMode {
    MergeWithActiveBase,
    ReplaceAll,
}

impl DraftApplicationMode {
    pub(super) fn expected_source_kind(self) -> &'static str {
        match self {
            Self::MergeWithActiveBase => "upload",
            Self::ReplaceAll => "case",
        }
    }
}

#[derive(Clone)]
pub struct UploadConfirmationRepo {
    pub(super) pool: DbPool,
    resources: DataResourceRepo,
}

impl UploadConfirmationRepo {
    pub fn new(pool: DbPool) -> Self {
        Self {
            resources: DataResourceRepo::new(pool.clone()),
            pool,
        }
    }

    pub async fn claim_confirmation(
        &self,
        user: &UserId,
        batch_id: i32,
        operation_id: &str,
        payload_hash: &str,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let operation_id = operation_id.to_string();
        let payload_hash = payload_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = data_resource::owned_batch(conn, &user, batch_id)?;
                if !matches!(
                    batch.status.as_str(),
                    "awaiting_purpose" | "ready_to_confirm" | "partially_confirmed" | "confirmed"
                ) {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                if let Some(existing) = batch.confirm_operation_id {
                    if existing == operation_id
                        && batch.confirm_payload_hash.as_deref() == Some(payload_hash.as_str())
                    {
                        return Ok(());
                    }
                    if existing == operation_id
                        || batch.confirmation_in_progress
                        || batch.status == "confirmed"
                    {
                        return Err(RepoError::InvalidState("idempotency_conflict".into()));
                    }
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::confirm_operation_id.eq(Some(operation_id)),
                        schema::upload_batches::confirm_payload_hash.eq(Some(payload_hash)),
                        schema::upload_batches::confirmation_started.eq(false),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn begin_confirmation(
        &self,
        user: &UserId,
        batch_id: i32,
        operation_id: &str,
        payload_hash: &str,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let operation_id = operation_id.to_string();
        let payload_hash = payload_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = data_resource::owned_batch(conn, &user, batch_id)?;
                if batch.confirm_operation_id.as_deref() != Some(operation_id.as_str())
                    || batch.confirm_payload_hash.as_deref() != Some(payload_hash.as_str())
                    || batch.confirmation_in_progress
                    || !matches!(
                        batch.status.as_str(),
                        "ready_to_confirm" | "partially_confirmed"
                    )
                {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::confirmation_started.eq(true),
                        schema::upload_batches::confirmation_in_progress.eq(true),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn end_confirmation(
        &self,
        user: &UserId,
        batch_id: i32,
        operation_id: &str,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let operation_id = operation_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let changed = diesel::update(
                schema::upload_batches::table
                    .filter(schema::upload_batches::id.eq(batch_id))
                    .filter(schema::upload_batches::confirm_operation_id.eq(operation_id))
                    .filter(schema::upload_batches::confirmation_in_progress.eq(true))
                    .filter(
                        schema::upload_batches::session_id.eq_any(
                            schema::sessions::table
                                .filter(schema::sessions::user_id.eq(user))
                                .filter(schema::sessions::deletion_status.eq("live"))
                                .select(schema::sessions::id),
                        ),
                    ),
            )
            .set((
                schema::upload_batches::confirmation_in_progress.eq(false),
                schema::upload_batches::updated_at_ms.eq(now_ms),
            ))
            .execute(&mut conn)?;
            if changed == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn publish_resources(
        &self,
        user: &UserId,
        batch_id: i32,
        inputs: Vec<ResourcePublishInput>,
        replacement: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<PublishChildResult> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                transactions::publish_resources_on(
                    conn,
                    &user,
                    batch_id,
                    inputs,
                    replacement,
                    now_ms,
                )
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn apply_model_candidates_to_draft(
        &self,
        user: &UserId,
        batch_id: i32,
        expected_draft: Option<DraftRef>,
        mode: DraftApplicationMode,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                transactions::apply_model_candidates_to_draft_on(
                    conn,
                    &user,
                    batch_id,
                    expected_draft,
                    mode,
                    now_ms,
                )
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn mark_child_failed(
        &self,
        user: &UserId,
        batch_id: i32,
        child: &str,
        diagnostic: String,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let child = child.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let batch = data_resource::owned_batch(&mut conn, &user, batch_id)?;
            if matches!(
                batch.status.as_str(),
                "discarded" | "expired" | "confirmed" | "failed"
            ) {
                return Ok(());
            }
            if child == "model" {
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::model_publish_status.eq("failed"),
                        schema::upload_batches::model_diagnostics_json.eq(diagnostic),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(&mut conn)?;
            } else {
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::resource_publish_status.eq("failed"),
                        schema::upload_batches::resource_diagnostics_json.eq(diagnostic),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(&mut conn)?;
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn finish_batch(
        &self,
        user: &UserId,
        batch_id: i32,
        now_ms: i64,
    ) -> RepoResult<FinishedUploadBatch> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = data_resource::owned_batch(conn, &user, batch_id)?;
                if matches!(
                    batch.status.as_str(),
                    "discarded" | "expired" | "confirmed" | "failed"
                ) {
                    return Ok(FinishedUploadBatch {
                        staged_paths: Vec::new(),
                    });
                }
                let model_done = matches!(
                    batch.model_publish_status.as_str(),
                    "published" | "not_applicable"
                );
                let resource_done = matches!(
                    batch.resource_publish_status.as_str(),
                    "published" | "not_applicable"
                );
                let status = settled_status(
                    &batch.model_publish_status,
                    &batch.resource_publish_status,
                );
                let retained_candidates = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(schema::upload_candidates::selected.eq(true))
                    .load::<UploadCandidateRecord>(conn)?
                    .into_iter()
                    .filter(|candidate| match candidate.purpose.as_deref() {
                        Some("model_table") => !model_done,
                        Some("data_resource") => !resource_done,
                        _ => false,
                    })
                    .collect::<Vec<_>>();
                let retained_candidate_ids = retained_candidates
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect::<BTreeSet<_>>();
                let retained_file_ids = retained_candidates
                    .into_iter()
                    .map(|candidate| candidate.file_id)
                    .collect::<BTreeSet<_>>();
                let files = schema::upload_files::table
                    .filter(schema::upload_files::batch_id.eq(batch_id))
                    .load::<UploadFileRecord>(conn)?;
                let retained_bytes = files
                    .iter()
                    .filter(|file| retained_file_ids.contains(&file.id))
                    .try_fold(0_i64, |total, file| total.checked_add(file.size_bytes))
                    .ok_or_else(|| RepoError::InvalidState("上传预留大小溢出".into()))?
                    .min(batch.reserved_bytes);
                let released_bytes = batch.reserved_bytes - retained_bytes;
                let paths = files
                    .iter()
                    .filter(|file| !retained_file_ids.contains(&file.id))
                    .map(|file| file.staged_path.clone())
                    .collect::<Vec<_>>();
                let mut release_artifact_ids = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(
                        schema::upload_candidates::id
                            .ne_all(retained_candidate_ids.iter().cloned()),
                    )
                    .filter(schema::upload_candidates::normalized_artifact_id.is_not_null())
                    .select(schema::upload_candidates::normalized_artifact_id)
                    .load::<Option<i32>>(conn)?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                release_artifact_ids.extend(
                    files
                        .iter()
                        .filter(|file| !retained_file_ids.contains(&file.id))
                        .filter_map(|file| file.source_artifact_id),
                );
                let scope_id = schema::sessions::table
                    .inner_join(
                        schema::users::table.on(schema::users::id.eq(schema::sessions::user_id)),
                    )
                    .filter(schema::sessions::id.eq(batch.session_id))
                    .filter(schema::sessions::user_id.eq(&user))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                super::artifact_refs::release_refs(
                    conn,
                    scope_id,
                    release_artifact_ids,
                    now_ms,
                )?;
                if !retained_candidate_ids.is_empty() {
                    diesel::update(
                        schema::upload_candidates::table
                            .filter(schema::upload_candidates::batch_id.eq(batch_id))
                            .filter(
                                schema::upload_candidates::id
                                    .ne_all(retained_candidate_ids.iter().cloned()),
                            ),
                    )
                    .set(
                        schema::upload_candidates::normalized_artifact_id
                            .eq(Option::<i32>::None),
                    )
                    .execute(conn)?;
                } else {
                    diesel::update(
                        schema::upload_candidates::table
                            .filter(schema::upload_candidates::batch_id.eq(batch_id)),
                    )
                    .set(
                        schema::upload_candidates::normalized_artifact_id
                            .eq(Option::<i32>::None),
                    )
                    .execute(conn)?;
                }
                if !retained_file_ids.is_empty() {
                    diesel::update(
                        schema::upload_files::table
                            .filter(schema::upload_files::batch_id.eq(batch_id))
                            .filter(
                                schema::upload_files::id
                                    .ne_all(retained_file_ids.iter().cloned()),
                            ),
                    )
                    .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                    .execute(conn)?;
                } else {
                    diesel::update(
                        schema::upload_files::table
                            .filter(schema::upload_files::batch_id.eq(batch_id)),
                    )
                    .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                    .execute(conn)?;
                }
                if released_bytes > 0 {
                    diesel::sql_query("UPDATE storage_scopes SET reserved_bytes=reserved_bytes-?1,updated_at_ms=?2 WHERE id=(SELECT u.storage_scope_id FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.id=?3)")
                        .bind::<BigInt, _>(released_bytes)
                        .bind::<BigInt, _>(now_ms)
                        .bind::<Integer, _>(batch.session_id)
                        .execute(conn)?;
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::status.eq(status),
                        schema::upload_batches::confirmation_in_progress.eq(false),
                        schema::upload_batches::reserved_bytes.eq(retained_bytes),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(FinishedUploadBatch {
                    staged_paths: paths,
                })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_current_for_user(
        &self,
        user: &UserId,
    ) -> RepoResult<Vec<DataResourceRecord>> {
        self.resources.list_current_for_user(user).await
    }
}
