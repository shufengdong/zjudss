use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use sha2::{Digest, Sha256};

use crate::db::DbPool;
use crate::db::models::EngineResourceSnapshotRecord;
use crate::db::schema::engine_resource_snapshots;
use crate::engine_resources::EngineResourceSnapshot;

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct EngineResourceSnapshotRepo {
    pool: DbPool,
}

impl EngineResourceSnapshotRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn publish_current(
        &self,
        snapshot: &EngineResourceSnapshot,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let record = snapshot_record(snapshot, now_ms)?;
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut connection = pool.get().map_err(RepoError::Pool)?;
            connection
                .transaction::<_, diesel::result::Error, _>(|connection| {
                    diesel::update(engine_resource_snapshots::table)
                        .set(engine_resource_snapshots::is_current.eq(false))
                        .execute(connection)?;
                    diesel::sql_query(
                        "INSERT INTO engine_resource_snapshots(
                       id,engine_contract_version,catalog_hash,
                       functions_schema_version,study_facades_schema_version,
                       dataframes_schema_version,dtype_grammar_version,is_current,
                       snapshot_json,created_at_ms
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9)
                     ON CONFLICT(id) DO UPDATE SET is_current=1",
                    )
                    .bind::<Text, _>(&record.id)
                    .bind::<Text, _>(&record.engine_contract_version)
                    .bind::<Text, _>(&record.catalog_hash)
                    .bind::<Integer, _>(record.functions_schema_version)
                    .bind::<Integer, _>(record.study_facades_schema_version)
                    .bind::<Integer, _>(record.dataframes_schema_version)
                    .bind::<Integer, _>(record.dtype_grammar_version)
                    .bind::<Text, _>(&record.snapshot_json)
                    .bind::<BigInt, _>(record.created_at_ms)
                    .execute(connection)?;
                    Ok(())
                })
                .map_err(RepoError::Query)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn current(&self) -> RepoResult<Option<EngineResourceSnapshotRecord>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(
            move || -> RepoResult<Option<EngineResourceSnapshotRecord>> {
                let mut connection = pool.get().map_err(RepoError::Pool)?;
                engine_resource_snapshots::table
                    .filter(engine_resource_snapshots::is_current.eq(true))
                    .first::<EngineResourceSnapshotRecord>(&mut connection)
                    .optional()
                    .map_err(RepoError::Query)
            },
        )
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn count(&self) -> RepoResult<i64> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<i64> {
            let mut connection = pool.get().map_err(RepoError::Pool)?;
            engine_resource_snapshots::table
                .count()
                .get_result(&mut connection)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn snapshot_record(
    snapshot: &EngineResourceSnapshot,
    now_ms: i64,
) -> RepoResult<EngineResourceSnapshotRecord> {
    let versions = snapshot.catalog_schema_versions;
    let hash_input = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        snapshot.dataframes_sha256,
        snapshot.functions_sha256,
        snapshot.study_facades_sha256,
        versions.functions,
        versions.study_facades,
        versions.dataframes,
        versions.dtype_grammar,
    );
    let catalog_hash = hex::encode(Sha256::digest(hash_input.as_bytes()));
    let snapshot_json = serde_json::to_string(&serde_json::json!({
        "snapshotId": snapshot.id,
        "engineContractVersion": snapshot.contract_version,
        "catalogSchemaVersions": versions,
        "catalogHashes": {
            "functions": snapshot.functions_sha256,
            "studyFacades": snapshot.study_facades_sha256,
            "dataframes": snapshot.dataframes_sha256,
        },
        "facades": {
            "catalogEntries": snapshot.facades.catalog_entry_count(),
            "enabled": snapshot.facades.enabled_count(),
            "diagnostics": snapshot.facades.diagnostics(),
        }
    }))
    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
    Ok(EngineResourceSnapshotRecord {
        id: snapshot.id.clone(),
        engine_contract_version: snapshot.contract_version.clone(),
        catalog_hash,
        functions_schema_version: i32::try_from(versions.functions)
            .map_err(|error| RepoError::InvalidState(error.to_string()))?,
        study_facades_schema_version: i32::try_from(versions.study_facades)
            .map_err(|error| RepoError::InvalidState(error.to_string()))?,
        dataframes_schema_version: i32::try_from(versions.dataframes)
            .map_err(|error| RepoError::InvalidState(error.to_string()))?,
        dtype_grammar_version: i32::try_from(versions.dtype_grammar)
            .map_err(|error| RepoError::InvalidState(error.to_string()))?,
        is_current: true,
        snapshot_json,
        created_at_ms: now_ms,
    })
}
