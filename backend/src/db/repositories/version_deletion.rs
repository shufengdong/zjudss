use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use sha2::{Digest, Sha256};

use crate::auth::UserId;
use crate::db::schema;

use super::{RepoError, VersionRepo, VersionRepoError, VersionRow, WorkspaceRow, scoped_version};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionImpact {
    pub run_ids: Vec<i32>,
    pub result_ids: Vec<i32>,
    pub model_dataframe_ids: Vec<i32>,
    pub reclaimable_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct DeleteResult {
    pub release_artifact_ids: Vec<i32>,
}

impl VersionRepo {
    pub async fn preview_deletion(
        &self,
        user: &UserId,
        version_id: i32,
        token_hash: String,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<DeletionImpact, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<DeletionImpact,VersionRepoError,_>(|conn| {
                let version = scoped_version(conn, &user, version_id, true)?;
                assert_deletable(conn, &version)?;
                let impact = deletion_impact(conn, version_id)?;
                let impact_json = serde_json::to_string(&impact).map_err(|error| VersionRepoError::Repository(RepoError::Storage(error.to_string())))?;
                let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
                diesel::sql_query("DELETE FROM version_deletion_previews WHERE expires_at_ms<?1 OR (user_id=?2 AND version_id=?3)")
                    .bind::<BigInt,_>(now_ms).bind::<Text,_>(&user).bind::<Integer,_>(version_id).execute(conn)?;
                diesel::sql_query("INSERT INTO version_deletion_previews(token_hash,user_id,version_id,impact_hash,impact_json,expires_at_ms,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                    .bind::<Text,_>(token_hash).bind::<Text,_>(user).bind::<Integer,_>(version_id).bind::<Text,_>(impact_hash).bind::<Text,_>(impact_json).bind::<BigInt,_>(expires_at_ms).bind::<BigInt,_>(now_ms).execute(conn)?;
                Ok(impact)
            })
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn delete_version(
        &self,
        user: &UserId,
        version_id: i32,
        token_hash: String,
        now_ms: i64,
    ) -> Result<DeleteResult, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<DeleteResult,VersionRepoError,_>(|conn| {
                #[derive(QueryableByName)] struct Preview { #[diesel(sql_type=Text)] impact_hash:String }
                let preview = diesel::sql_query("SELECT impact_hash FROM version_deletion_previews WHERE token_hash=?1 AND user_id=?2 AND version_id=?3 AND expires_at_ms>=?4")
                    .bind::<Text,_>(&token_hash).bind::<Text,_>(&user).bind::<Integer,_>(version_id).bind::<BigInt,_>(now_ms).get_result::<Preview>(conn).optional()?;
                let Some(preview) = preview else { return Err(VersionRepoError::InvalidToken); };
                let version = scoped_version(conn, &user, version_id, true)?;
                assert_deletable(conn, &version)?;
                let impact = deletion_impact(conn, version_id)?;
                let impact_json = serde_json::to_string(&impact).map_err(|error| VersionRepoError::Repository(RepoError::Storage(error.to_string())))?;
                let actual_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
                if actual_hash != preview.impact_hash { return Err(VersionRepoError::ImpactChanged); }
                #[derive(QueryableByName)] struct ArtifactId { #[diesel(sql_type=Integer)] artifact_id:i32 }
                let mut release = diesel::sql_query("SELECT artifact_id FROM model_dataframes WHERE version_id=?1 ORDER BY id")
                    .bind::<Integer,_>(version_id).load::<ArtifactId>(conn)?.into_iter().map(|row| row.artifact_id).collect::<Vec<_>>();
                release.extend(diesel::sql_query("SELECT d.artifact_id FROM result_datasets d JOIN simulation_runs r ON r.id=d.run_id WHERE r.model_version_id=?1 AND d.deleted_at_ms IS NULL ORDER BY d.id")
                    .bind::<Integer,_>(version_id).load::<ArtifactId>(conn)?.into_iter().map(|row| row.artifact_id));
                let tombstone = serde_json::json!({"contentHash":version.content_hash,"impact":impact,"deletedAtMs":now_ms}).to_string();
                diesel::delete(schema::model_dataframes::table.filter(schema::model_dataframes::version_id.eq(version_id))).execute(conn)?;
                diesel::sql_query("UPDATE result_datasets SET artifact_id=NULL,deleted_at_ms=?1,tombstone_json=json_object('resultName',result_name,'schemaHash',schema_hash,'rowCount',row_count,'sizeBytes',size_bytes) WHERE run_id IN (SELECT id FROM simulation_runs WHERE model_version_id=?2) AND deleted_at_ms IS NULL")
                    .bind::<BigInt,_>(now_ms).bind::<Integer,_>(version_id).execute(conn)?;
                diesel::sql_query("UPDATE simulation_runs SET deleted_at_ms=?1,tombstone_json=json_object('clientRunId',client_run_id,'status',status,'modelContentHash',model_content_hash,'scriptContentHash',script_content_hash) WHERE model_version_id=?2 AND deleted_at_ms IS NULL")
                    .bind::<BigInt,_>(now_ms).bind::<Integer,_>(version_id).execute(conn)?;
                diesel::sql_query("UPDATE model_versions SET deleted_at_ms=?1,tombstone_json=?2 WHERE id=?3")
                    .bind::<BigInt,_>(now_ms).bind::<Text,_>(tombstone).bind::<Integer,_>(version_id).execute(conn)?;
                diesel::sql_query("DELETE FROM version_deletion_previews WHERE token_hash=?1").bind::<Text,_>(token_hash).execute(conn)?;
                Ok(DeleteResult { release_artifact_ids: release })
            })
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }
}

fn assert_deletable(
    conn: &mut SqliteConnection,
    version: &VersionRow,
) -> Result<(), VersionRepoError> {
    let workspace = diesel::sql_query("SELECT id,session_id,active_version_id,created_at_ms,updated_at_ms FROM workspaces WHERE id=?1").bind::<Integer,_>(version.workspace_id).get_result::<WorkspaceRow>(conn)?;
    if workspace.active_version_id == Some(version.id) {
        return Err(VersionRepoError::ActiveVersion);
    }
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type=BigInt)]
        count: i64,
    }
    let busy = diesel::sql_query("SELECT count(*) count FROM simulation_runs WHERE model_version_id=?1 AND deleted_at_ms IS NULL AND status IN ('queued','running')").bind::<Integer,_>(version.id).get_result::<Count>(conn)?.count;
    if busy > 0 {
        return Err(VersionRepoError::Busy);
    }
    Ok(())
}

fn deletion_impact(
    conn: &mut SqliteConnection,
    version_id: i32,
) -> Result<DeletionImpact, VersionRepoError> {
    #[derive(QueryableByName)]
    struct Id {
        #[diesel(sql_type=Integer)]
        id: i32,
    }
    #[derive(QueryableByName)]
    struct Bytes {
        #[diesel(sql_type=BigInt)]
        bytes: i64,
    }
    let run_ids = diesel::sql_query("SELECT id FROM simulation_runs WHERE model_version_id=?1 AND deleted_at_ms IS NULL ORDER BY id").bind::<Integer,_>(version_id).load::<Id>(conn)?.into_iter().map(|row|row.id).collect();
    let result_ids = diesel::sql_query("SELECT d.id FROM result_datasets d JOIN simulation_runs r ON r.id=d.run_id WHERE r.model_version_id=?1 AND d.deleted_at_ms IS NULL ORDER BY d.id").bind::<Integer,_>(version_id).load::<Id>(conn)?.into_iter().map(|row|row.id).collect();
    let model_dataframe_ids =
        diesel::sql_query("SELECT id FROM model_dataframes WHERE version_id=?1 ORDER BY id")
            .bind::<Integer, _>(version_id)
            .load::<Id>(conn)?
            .into_iter()
            .map(|row| row.id)
            .collect();
    let reclaimable_bytes = diesel::sql_query("SELECT COALESCE((SELECT sum(a.size_bytes) FROM model_dataframes md JOIN artifacts a ON a.id=md.artifact_id WHERE md.version_id=?1),0)+COALESCE((SELECT sum(d.size_bytes) FROM result_datasets d JOIN simulation_runs r ON r.id=d.run_id WHERE r.model_version_id=?1 AND d.deleted_at_ms IS NULL),0) bytes")
        .bind::<Integer,_>(version_id).get_result::<Bytes>(conn)?.bytes;
    Ok(DeletionImpact {
        run_ids,
        result_ids,
        model_dataframe_ids,
        reclaimable_bytes,
    })
}
