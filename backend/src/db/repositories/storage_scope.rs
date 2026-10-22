use diesel::prelude::*;

use crate::auth::StorageScopeRef;
use crate::db::models::{NewStorageScope, StorageScope};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageLimits {
    pub artifact_limit: i64,
    pub byte_limit: i64,
    pub warning_ratio: f64,
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            artifact_limit: 20_000,
            byte_limit: 20 * 1024 * 1024 * 1024,
            warning_ratio: 0.8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageUsage {
    pub artifact_limit: i64,
    pub byte_limit: i64,
    pub warning_ratio: f64,
    pub used_artifacts: i64,
    pub used_bytes: i64,
}

impl From<StorageScope> for StorageUsage {
    fn from(scope: StorageScope) -> Self {
        Self {
            artifact_limit: scope.artifact_limit,
            byte_limit: scope.byte_limit,
            warning_ratio: scope.warning_ratio,
            used_artifacts: scope.used_artifacts,
            used_bytes: scope.used_bytes,
        }
    }
}

#[derive(Clone)]
pub struct StorageScopeRepo {
    pool: DbPool,
    defaults: StorageLimits,
}

impl StorageScopeRepo {
    pub fn new(pool: DbPool, defaults: StorageLimits) -> Self {
        Self { pool, defaults }
    }

    pub async fn resolve(
        &self,
        reference: &StorageScopeRef,
        now_ms: i64,
    ) -> RepoResult<StorageScope> {
        let pool = self.pool.clone();
        let reference = reference.clone();
        let defaults = self.defaults;
        tokio::task::spawn_blocking(move || -> RepoResult<StorageScope> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<StorageScope, RepoError, _>(|conn| {
                diesel::insert_into(schema::storage_scopes::table)
                    .values(NewStorageScope {
                        kind: reference.kind.clone(),
                        scope_key: reference.key.clone(),
                        created_at_ms: now_ms,
                    })
                    .on_conflict((
                        schema::storage_scopes::kind,
                        schema::storage_scopes::scope_key,
                    ))
                    .do_nothing()
                    .execute(conn)?;
                diesel::update(
                    schema::storage_scopes::table
                        .filter(schema::storage_scopes::kind.eq(&reference.kind))
                        .filter(schema::storage_scopes::scope_key.eq(&reference.key))
                        .filter(schema::storage_scopes::updated_at_ms.eq(0)),
                )
                .set((
                    schema::storage_scopes::artifact_limit.eq(defaults.artifact_limit),
                    schema::storage_scopes::byte_limit.eq(defaults.byte_limit),
                    schema::storage_scopes::warning_ratio.eq(defaults.warning_ratio),
                    schema::storage_scopes::updated_at_ms.eq(now_ms),
                ))
                .execute(conn)?;
                schema::storage_scopes::table
                    .filter(schema::storage_scopes::kind.eq(reference.kind))
                    .filter(schema::storage_scopes::scope_key.eq(reference.key))
                    .first(conn)
                    .map_err(RepoError::Query)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn usage(&self, scope_id: i32) -> RepoResult<StorageUsage> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<StorageUsage> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::storage_scopes::table
                .find(scope_id)
                .first::<StorageScope>(&mut conn)
                .map(StorageUsage::from)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_limits(
        &self,
        scope_id: i32,
        limits: StorageLimits,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            if limits.artifact_limit <= 0
                || limits.byte_limit <= 0
                || !(0.0 < limits.warning_ratio && limits.warning_ratio <= 1.0)
            {
                return Err(RepoError::Query(diesel::result::Error::RollbackTransaction));
            }
            let changed = diesel::update(schema::storage_scopes::table.find(scope_id))
                .set((
                    schema::storage_scopes::artifact_limit.eq(limits.artifact_limit),
                    schema::storage_scopes::byte_limit.eq(limits.byte_limit),
                    schema::storage_scopes::warning_ratio.eq(limits.warning_ratio),
                    schema::storage_scopes::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            if changed == 0 {
                Err(RepoError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
        .map_err(RepoError::Join)?
    }
}
