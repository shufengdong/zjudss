use diesel::prelude::*;

use crate::db::models::{
    Artifact, ArtifactGcFailure, NewArtifact, StorageScope, StorageScopeArtifact,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

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
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Artifact> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let mut publish = Some(publish);
            conn.immediate_transaction::<Artifact, RepoError, _>(|conn| {
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
                let link = schema::storage_scope_artifacts::table
                    .find((scope_id, artifact_id))
                    .first::<StorageScopeArtifact>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if link.ref_count > 1 {
                    diesel::update(
                        schema::storage_scope_artifacts::table.find((scope_id, artifact_id)),
                    )
                    .set(schema::storage_scope_artifacts::ref_count.eq(link.ref_count - 1))
                    .execute(conn)?;
                    return Ok(());
                }
                let artifact = schema::artifacts::table
                    .find(artifact_id)
                    .first::<Artifact>(conn)?;
                diesel::delete(
                    schema::storage_scope_artifacts::table.find((scope_id, artifact_id)),
                )
                .execute(conn)?;
                diesel::update(schema::storage_scopes::table.find(scope_id))
                    .set((
                        schema::storage_scopes::used_artifacts
                            .eq(schema::storage_scopes::used_artifacts - 1),
                        schema::storage_scopes::used_bytes
                            .eq(schema::storage_scopes::used_bytes - artifact.size_bytes),
                        schema::storage_scopes::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
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
