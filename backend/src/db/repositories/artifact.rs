use diesel::prelude::*;
use std::collections::BTreeMap;

use crate::db::models::{
    Artifact, ArtifactGcFailure, NewArtifact, StorageScope, StorageScopeArtifact,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

pub(super) fn release_refs(
    conn: &mut SqliteConnection,
    scope_id: i32,
    artifact_ids: impl IntoIterator<Item = i32>,
    now_ms: i64,
) -> RepoResult<()> {
    let mut counts = BTreeMap::<i32, i64>::new();
    for artifact_id in artifact_ids {
        *counts.entry(artifact_id).or_default() += 1;
    }
    let mut removed_artifacts = 0_i64;
    let mut removed_bytes = 0_i64;
    for (artifact_id, release_count) in counts {
        let link = schema::storage_scope_artifacts::table
            .find((scope_id, artifact_id))
            .first::<StorageScopeArtifact>(conn)
            .optional()?
            .ok_or(RepoError::NotFound)?;
        if release_count > link.ref_count {
            return Err(RepoError::InvalidState(
                "artifact 引用释放次数超过持有次数".into(),
            ));
        }
        if release_count < link.ref_count {
            diesel::update(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
                .set(schema::storage_scope_artifacts::ref_count.eq(link.ref_count - release_count))
                .execute(conn)?;
        } else {
            let size_bytes = schema::artifacts::table
                .find(artifact_id)
                .select(schema::artifacts::size_bytes)
                .first::<i64>(conn)?;
            diesel::delete(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
                .execute(conn)?;
            removed_artifacts += 1;
            removed_bytes = removed_bytes
                .checked_add(size_bytes)
                .ok_or_else(|| RepoError::InvalidState("artifact 大小溢出".into()))?;
        }
    }
    if removed_artifacts > 0 {
        diesel::update(schema::storage_scopes::table.find(scope_id))
            .set((
                schema::storage_scopes::used_artifacts
                    .eq(schema::storage_scopes::used_artifacts - removed_artifacts),
                schema::storage_scopes::used_bytes
                    .eq(schema::storage_scopes::used_bytes - removed_bytes),
                schema::storage_scopes::updated_at_ms.eq(now_ms),
            ))
            .execute(conn)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ArtifactDraft {
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub format: String,
    pub media_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Clone)]
pub struct ArtifactRepo {
    pool: DbPool,
}

impl ArtifactRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn find_owned(&self, scope_id: i32, artifact_id: i32) -> RepoResult<Artifact> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Artifact> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::artifacts::table
                .inner_join(schema::storage_scope_artifacts::table)
                .filter(schema::artifacts::id.eq(artifact_id))
                .filter(schema::storage_scope_artifacts::storage_scope_id.eq(scope_id))
                .select(schema::artifacts::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn commit_with_publish<F>(
        &self,
        scope_id: i32,
        draft: ArtifactDraft,
        publish: F,
    ) -> RepoResult<Artifact>
    where
        F: FnOnce(&str) -> Result<(), RepoError> + Send + 'static,
    {
        self.commit_with_publish_for_upload(scope_id, draft, None, publish)
            .await
    }

    pub(crate) async fn commit_batch_with_publish<T, P, F>(
        &self,
        scope_id: i32,
        drafts: Vec<ArtifactDraft>,
        mut publish: P,
        finalize: F,
    ) -> RepoResult<T>
    where
        T: Send + 'static,
        P: FnMut(usize, &str) -> Result<(), RepoError> + Send + 'static,
        F: FnOnce(&mut SqliteConnection, &[Artifact]) -> RepoResult<T> + Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<T> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let mut finalize = Some(finalize);
            conn.immediate_transaction::<T, RepoError, _>(|conn| {
                let mut artifacts = Vec::with_capacity(drafts.len());
                for (index, draft) in drafts.iter().enumerate() {
                    artifacts.push(commit_draft_in_transaction(
                        conn,
                        scope_id,
                        draft,
                        |path| publish(index, path),
                    )?);
                }
                finalize.take().expect("finalize once")(conn, &artifacts)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn commit_upload_source_with_publish<F>(
        &self,
        scope_id: i32,
        draft: ArtifactDraft,
        upload_file_id: String,
        publish: F,
    ) -> RepoResult<Artifact>
    where
        F: FnOnce(&str) -> Result<(), RepoError> + Send + 'static,
    {
        self.commit_with_publish_for_upload(scope_id, draft, Some(upload_file_id), publish)
            .await
    }

    async fn commit_with_publish_for_upload<F>(
        &self,
        scope_id: i32,
        draft: ArtifactDraft,
        upload_file_id: Option<String>,
        publish: F,
    ) -> RepoResult<Artifact>
    where
        F: FnOnce(&str) -> Result<(), RepoError> + Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Artifact> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let mut publish = Some(publish);
            conn.immediate_transaction::<Artifact, RepoError, _>(|conn| {
                if let Some(file_id) = upload_file_id.as_deref() {
                    let (existing_source, batch_status, resource_publish_status) =
                        schema::upload_files::table
                        .inner_join(
                            schema::upload_batches::table
                                .on(schema::upload_batches::id.eq(schema::upload_files::batch_id)),
                        )
                        .inner_join(
                            schema::sessions::table
                                .on(schema::sessions::id.eq(schema::upload_batches::session_id)),
                        )
                        .inner_join(
                            schema::users::table
                                .on(schema::users::id.eq(schema::sessions::user_id)),
                        )
                        .filter(schema::upload_files::id.eq(file_id))
                        .filter(schema::users::storage_scope_id.eq(scope_id))
                        .filter(schema::sessions::deletion_status.eq("live"))
                        .select((
                            schema::upload_files::source_artifact_id,
                            schema::upload_batches::status,
                            schema::upload_batches::resource_publish_status,
                        ))
                        .first::<(Option<i32>, String, String)>(conn)
                        .optional()?
                        .ok_or(RepoError::NotFound)?;
                    if !matches!(
                        batch_status.as_str(),
                        "ready_to_confirm" | "partially_confirmed"
                    ) || (resource_publish_status == "published" && existing_source.is_none())
                    {
                        return Err(RepoError::InvalidState("upload_state_invalid".into()));
                    }
                    if let Some(artifact_id) = existing_source {
                        let artifact = schema::artifacts::table
                            .find(artifact_id)
                            .first::<Artifact>(conn)?;
                        if artifact.sha256 != draft.sha256
                            || artifact.size_bytes != draft.size_bytes
                        {
                            return Err(RepoError::InvalidState(
                                "上传原文件 artifact 与暂存内容不一致".into(),
                            ));
                        }
                        publish.take().expect("publish once")(&artifact.storage_path)?;
                        return Ok(artifact);
                    }
                }
                let scope = schema::storage_scopes::table
                    .find(scope_id)
                    .first::<StorageScope>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                let existing = schema::artifacts::table
                    .filter(schema::artifacts::sha256.eq(&draft.sha256))
                    .first::<Artifact>(conn)
                    .optional()?;
                if let Some(ref artifact) = existing {
                    if artifact.size_bytes != draft.size_bytes {
                        return Err(RepoError::Storage("相同哈希对应不同大小".to_string()));
                    }
                    if let Some(link) = schema::storage_scope_artifacts::table
                        .find((scope_id, artifact.id))
                        .first::<StorageScopeArtifact>(conn)
                        .optional()?
                    {
                        publish.take().expect("publish once")(&artifact.storage_path)?;
                        diesel::update(
                            schema::storage_scope_artifacts::table.find((scope_id, artifact.id)),
                        )
                        .set(schema::storage_scope_artifacts::ref_count.eq(link.ref_count + 1))
                        .execute(conn)?;
                        if let Some(file_id) = upload_file_id.as_deref() {
                            diesel::update(schema::upload_files::table.find(file_id))
                                .set(schema::upload_files::source_artifact_id.eq(Some(artifact.id)))
                                .execute(conn)?;
                        }
                        return Ok(artifact.clone());
                    }
                }
                if scope.used_artifacts >= scope.artifact_limit
                    || scope
                        .used_bytes
                        .checked_add(draft.size_bytes)
                        .is_none_or(|bytes| bytes > scope.byte_limit)
                {
                    return Err(RepoError::QuotaExceeded);
                }
                let storage_path = existing
                    .as_ref()
                    .map(|a| a.storage_path.as_str())
                    .unwrap_or(&draft.storage_path);
                publish.take().expect("publish once")(storage_path)?;
                let artifact = match existing {
                    Some(artifact) => artifact,
                    None => {
                        diesel::insert_into(schema::artifacts::table)
                            .values(NewArtifact {
                                sha256: draft.sha256,
                                size_bytes: draft.size_bytes,
                                storage_path: draft.storage_path,
                                format: draft.format,
                                media_type: draft.media_type,
                                created_at_ms: draft.created_at_ms,
                            })
                            .execute(conn)?;
                        schema::artifacts::table
                            .order(schema::artifacts::id.desc())
                            .first(conn)?
                    }
                };
                diesel::insert_into(schema::storage_scope_artifacts::table)
                    .values(StorageScopeArtifact {
                        storage_scope_id: scope_id,
                        artifact_id: artifact.id,
                        ref_count: 1,
                        created_at_ms: draft.created_at_ms,
                    })
                    .execute(conn)?;
                diesel::update(schema::storage_scopes::table.find(scope_id))
                    .set((
                        schema::storage_scopes::used_artifacts.eq(scope.used_artifacts + 1),
                        schema::storage_scopes::used_bytes
                            .eq(scope.used_bytes + artifact.size_bytes),
                        schema::storage_scopes::updated_at_ms.eq(draft.created_at_ms),
                    ))
                    .execute(conn)?;
                if let Some(file_id) = upload_file_id.as_deref() {
                    diesel::update(schema::upload_files::table.find(file_id))
                        .set(schema::upload_files::source_artifact_id.eq(Some(artifact.id)))
                        .execute(conn)?;
                }
                Ok(artifact)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn release(&self, scope_id: i32, artifact_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(), RepoError, _>(|conn| {
                release_refs(conn, scope_id, [artifact_id], now_ms)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn take_unreferenced(&self) -> RepoResult<Vec<Artifact>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Artifact>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Vec<Artifact>, RepoError, _>(|conn| {
                let rows = schema::artifacts::table
                    .left_join(schema::storage_scope_artifacts::table)
                    .filter(schema::storage_scope_artifacts::artifact_id.is_null())
                    .select(schema::artifacts::all_columns)
                    .load::<Artifact>(conn)?;
                for artifact in &rows {
                    diesel::delete(schema::artifacts::table.find(artifact.id)).execute(conn)?;
                }
                Ok(rows)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn storage_paths(&self) -> RepoResult<Vec<String>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<String>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::artifacts::table
                .select(schema::artifacts::storage_path)
                .load(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn record_gc_failure(
        &self,
        relative_path: String,
        error: String,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query("INSERT INTO artifact_gc_failures(relative_path,error,attempts,updated_at_ms) VALUES (?1,?2,1,?3) ON CONFLICT(relative_path) DO UPDATE SET error=excluded.error, attempts=attempts+1, updated_at_ms=excluded.updated_at_ms")
                .bind::<diesel::sql_types::Text,_>(relative_path)
                .bind::<diesel::sql_types::Text,_>(error)
                .bind::<diesel::sql_types::BigInt,_>(now_ms)
                .execute(&mut conn)?;
            Ok(())
        }).await.map_err(RepoError::Join)?
    }

    pub async fn gc_failures(&self) -> RepoResult<Vec<ArtifactGcFailure>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<ArtifactGcFailure>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::artifact_gc_failures::table
                .order(schema::artifact_gc_failures::updated_at_ms.asc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn clear_gc_failure(&self, relative_path: String) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::delete(schema::artifact_gc_failures::table.find(relative_path))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn commit_draft_in_transaction<F>(
    conn: &mut SqliteConnection,
    scope_id: i32,
    draft: &ArtifactDraft,
    publish: F,
) -> RepoResult<Artifact>
where
    F: FnOnce(&str) -> Result<(), RepoError>,
{
    let scope = schema::storage_scopes::table
        .find(scope_id)
        .first::<StorageScope>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    let existing = schema::artifacts::table
        .filter(schema::artifacts::sha256.eq(&draft.sha256))
        .first::<Artifact>(conn)
        .optional()?;
    if let Some(ref artifact) = existing {
        if artifact.size_bytes != draft.size_bytes {
            return Err(RepoError::Storage("相同哈希对应不同大小".to_string()));
        }
        if let Some(link) = schema::storage_scope_artifacts::table
            .find((scope_id, artifact.id))
            .first::<StorageScopeArtifact>(conn)
            .optional()?
        {
            publish(&artifact.storage_path)?;
            diesel::update(schema::storage_scope_artifacts::table.find((scope_id, artifact.id)))
                .set(schema::storage_scope_artifacts::ref_count.eq(link.ref_count + 1))
                .execute(conn)?;
            return Ok(artifact.clone());
        }
    }
    if scope.used_artifacts >= scope.artifact_limit
        || scope
            .used_bytes
            .checked_add(draft.size_bytes)
            .is_none_or(|bytes| bytes > scope.byte_limit)
    {
        return Err(RepoError::QuotaExceeded);
    }
    let storage_path = existing
        .as_ref()
        .map(|artifact| artifact.storage_path.as_str())
        .unwrap_or(&draft.storage_path);
    publish(storage_path)?;
    let artifact = match existing {
        Some(artifact) => artifact,
        None => {
            diesel::insert_into(schema::artifacts::table)
                .values(NewArtifact {
                    sha256: draft.sha256.clone(),
                    size_bytes: draft.size_bytes,
                    storage_path: draft.storage_path.clone(),
                    format: draft.format.clone(),
                    media_type: draft.media_type.clone(),
                    created_at_ms: draft.created_at_ms,
                })
                .execute(conn)?;
            schema::artifacts::table
                .order(schema::artifacts::id.desc())
                .first(conn)?
        }
    };
    diesel::insert_into(schema::storage_scope_artifacts::table)
        .values(StorageScopeArtifact {
            storage_scope_id: scope_id,
            artifact_id: artifact.id,
            ref_count: 1,
            created_at_ms: draft.created_at_ms,
        })
        .execute(conn)?;
    diesel::update(schema::storage_scopes::table.find(scope_id))
        .set((
            schema::storage_scopes::used_artifacts.eq(scope.used_artifacts + 1),
            schema::storage_scopes::used_bytes.eq(scope.used_bytes + artifact.size_bytes),
            schema::storage_scopes::updated_at_ms.eq(draft.created_at_ms),
        ))
        .execute(conn)?;
    Ok(artifact)
}
