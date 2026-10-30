use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use sha2::{Digest, Sha256};

use crate::auth::UserId;
use crate::db::models::{NewSession, Session};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionDeletionImpact {
    pub model_count: i64,
    pub data_resource_count: i64,
    pub script_count: i64,
    pub run_count: i64,
    pub result_count: i64,
    pub analysis_artifact_count: i64,
    pub logical_bytes: i64,
    upload_batch_count: i64,
    replacement_preview_count: i64,
    impact_fingerprint: String,
}

#[derive(QueryableByName)]
struct SessionDeletionImpactRow {
    #[diesel(sql_type = BigInt)]
    model_count: i64,
    #[diesel(sql_type = BigInt)]
    data_resource_count: i64,
    #[diesel(sql_type = BigInt)]
    script_count: i64,
    #[diesel(sql_type = BigInt)]
    run_count: i64,
    #[diesel(sql_type = BigInt)]
    result_count: i64,
    #[diesel(sql_type = BigInt)]
    analysis_artifact_count: i64,
    #[diesel(sql_type = BigInt)]
    logical_bytes: i64,
    #[diesel(sql_type = BigInt)]
    upload_batch_count: i64,
    #[diesel(sql_type = BigInt)]
    replacement_preview_count: i64,
    #[diesel(sql_type = Text)]
    impact_fingerprint: String,
}

#[derive(Clone)]
pub struct SessionRepo {
    pool: DbPool,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, user_id: &UserId, title: &str, now_ms: i64) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = if title.trim().is_empty() {
            "新会话".to_string()
        } else {
            title.to_string()
        };
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Session, diesel::result::Error, _>(|conn| {
                diesel::insert_into(schema::sessions::table)
                    .values(NewSession {
                        title,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                        user_id: user_id.0.clone(),
                    })
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::sessions::table.find(id).first(conn)
            })
            .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(&self, user_id: &UserId) -> RepoResult<Vec<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::user_id.eq(user_id.0))
                .filter(schema::sessions::deletion_status.eq("live"))
                .order(schema::sessions::updated_at_ms.desc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn touch(&self, user_id: &UserId, session_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let updated = diesel::update(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(user_id.0))
                    .filter(schema::sessions::deletion_status.eq("live")),
            )
            .set(schema::sessions::updated_at_ms.eq(now_ms))
            .execute(&mut conn)
            .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user_id: &UserId, session_id: i32) -> RepoResult<Option<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .filter(schema::sessions::deletion_status.eq("live"))
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        user_id: &UserId,
        session_id: i32,
        title: &str,
        now_ms: i64,
    ) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let target = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(&user_id.0))
                .filter(schema::sessions::deletion_status.eq("live"));
            let updated = diesel::update(target)
                .set((
                    schema::sessions::title.eq(title),
                    schema::sessions::title_source.eq("user"),
                    schema::sessions::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .filter(schema::sessions::deletion_status.eq("live"))
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn preview_deletion(
        &self,
        user_id: &UserId,
        session_id: i32,
        token_hash: String,
        expires_at_ms: i64,
        created_at_ms: i64,
    ) -> RepoResult<SessionDeletionImpact> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<SessionDeletionImpact> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<SessionDeletionImpact, RepoError, _>(|conn| {
                require_live_session(conn, &user_id, session_id)?;
                let impact = session_deletion_impact(conn, session_id)?;
                let impact_json = serde_json::to_string(&impact)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
                let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
                diesel::insert_into(schema::session_deletion_previews::table)
                    .values(crate::db::models::SessionDeletionPreviewRecord {
                        token_hash,
                        user_id: user_id.0.clone(),
                        session_id,
                        impact_hash,
                        impact_json,
                        expires_at_ms,
                        created_at_ms,
                    })
                    .execute(conn)?;
                Ok(impact)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// Atomically verifies the preview and closes the session to all new writes.
    /// Returns `false` when the owned session was already deleted.
    pub async fn begin_delete(
        &self,
        user_id: &UserId,
        session_id: i32,
        token_hash: String,
        now_ms: i64,
    ) -> RepoResult<bool> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<bool> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<bool, RepoError, _>(|conn| {
                let status = schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(&user_id.0))
                    .select(schema::sessions::deletion_status)
                    .first::<String>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if status == "deleted" {
                    return Ok(false);
                }
                if status == "deleting" {
                    let authorized = schema::session_deletion_previews::table
                        .filter(schema::session_deletion_previews::token_hash.eq(&token_hash))
                        .filter(schema::session_deletion_previews::user_id.eq(&user_id.0))
                        .filter(schema::session_deletion_previews::session_id.eq(session_id))
                        .select(schema::session_deletion_previews::token_hash)
                        .first::<String>(conn)
                        .optional()?
                        .is_some();
                    if authorized {
                        return Ok(true);
                    }
                    return Err(RepoError::InvalidState("session_deleting".into()));
                }
                if status != "live" {
                    return Err(RepoError::InvalidState("session_deleting".into()));
                }
                let preview = schema::session_deletion_previews::table
                    .filter(schema::session_deletion_previews::token_hash.eq(&token_hash))
                    .filter(schema::session_deletion_previews::user_id.eq(&user_id.0))
                    .filter(schema::session_deletion_previews::session_id.eq(session_id))
                    .filter(schema::session_deletion_previews::expires_at_ms.ge(now_ms))
                    .first::<crate::db::models::SessionDeletionPreviewRecord>(conn)
                    .optional()?
                    .ok_or_else(|| {
                        RepoError::InvalidState("invalid_deletion_impact_token".into())
                    })?;
                let impact = session_deletion_impact(conn, session_id)?;
                let impact_json = serde_json::to_string(&impact)
                    .map_err(|error| RepoError::InvalidState(error.to_string()))?;
                let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
                if impact_hash != preview.impact_hash {
                    return Err(RepoError::InvalidState("deletion_impact_changed".into()));
                }
                diesel::update(
                    schema::data_resources::table
                        .filter(schema::data_resources::session_id.eq(session_id))
                        .filter(schema::data_resources::replaced_by_resource_id.is_not_null()),
                )
                .set(schema::data_resources::replaced_by_resource_id.eq(Option::<String>::None))
                .execute(conn)?;
                diesel::update(schema::sessions::table.find(session_id))
                    .set((
                        schema::sessions::deletion_status.eq("deleting"),
                        schema::sessions::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(true)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(&self, user_id: &UserId, session_id: i32) -> RepoResult<Vec<i32>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<i32>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Vec<i32>, RepoError, _>(|conn| {
                let status = schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(&user_id.0))
                    .select(schema::sessions::deletion_status)
                    .first::<String>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if status == "deleted" {
                    return Ok(Vec::new());
                }
                if status != "live" && status != "deleting" {
                    return Err(RepoError::InvalidState("session_deleting".into()));
                }
                if status == "deleting" {
                    // The immediate transaction keeps the session hidden as `deleting` from other
                    // connections. Locally restoring `live` lets FK cleanup perform its required
                    // SET NULL updates without tripping the write-blocking live-session triggers.
                    diesel::update(schema::sessions::table.find(session_id))
                        .set(schema::sessions::deletion_status.eq("live"))
                        .execute(conn)?;
                }

                #[derive(QueryableByName)]
                struct ArtifactId {
                    #[diesel(sql_type = diesel::sql_types::Integer)]
                    artifact_id: i32,
                }
                let artifact_ids = diesel::sql_query(
                    "SELECT artifact_id FROM (
                       SELECT d.artifact_id AS artifact_id
                       FROM draft_dataframes d
                       JOIN workspace_drafts wd ON wd.id=d.draft_id
                       JOIN workspaces w ON w.id=wd.workspace_id
                       WHERE w.session_id=?1
                       UNION ALL
                       SELECT d.artifact_id
                       FROM model_dataframes d
                       JOIN model_versions v ON v.id=d.version_id
                       JOIN workspaces w ON w.id=v.workspace_id
                       WHERE w.session_id=?1
                       UNION ALL
                       SELECT c.normalized_artifact_id FROM upload_candidates c
                       JOIN upload_batches b ON b.id=c.batch_id
                       WHERE b.session_id=?1 AND c.normalized_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT f.source_artifact_id FROM upload_files f
                       JOIN upload_batches b ON b.id=f.batch_id
                       WHERE b.session_id=?1 AND f.source_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT normalized_artifact_id FROM data_resources
                       WHERE session_id=?1 AND normalized_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT source_artifact_id FROM data_resources WHERE session_id=?1 AND source_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT p.pending_artifact_id FROM data_resource_replacement_previews p
                       JOIN data_resources d ON d.id=p.old_resource_id WHERE d.session_id=?1
                       UNION ALL
                       SELECT artifact_id FROM script_resources WHERE session_id=?1
                       UNION ALL
                       SELECT script_artifact_id FROM simulation_runs WHERE session_id=?1 AND script_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT CAST(json_extract(j.value,'$.artifactId') AS INTEGER)
                       FROM simulation_runs r,json_each(r.input_snapshot_json,'$.model.frames') j
                       WHERE r.session_id=?1 AND json_extract(j.value,'$.artifactId') IS NOT NULL
                       UNION ALL
                       SELECT CAST(json_extract(j.value,'$.artifactId') AS INTEGER)
                       FROM simulation_runs r,json_each(r.input_snapshot_json,'$.resources') j
                       WHERE r.session_id=?1 AND json_extract(j.value,'$.artifactId') IS NOT NULL
                       UNION ALL
                       SELECT i.artifact_id FROM workflow_input_snapshots i
                       JOIN managed_workflows w ON w.id=i.workflow_id
                       WHERE w.session_id=?1 AND i.artifact_id IS NOT NULL
                       UNION ALL
                       SELECT a.script_artifact_id FROM workflow_attempts a
                       JOIN managed_workflows w ON w.id=a.workflow_id
                       WHERE w.session_id=?1 AND a.script_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT a.stderr_audit_artifact_id FROM workflow_attempts a
                       JOIN managed_workflows w ON w.id=a.workflow_id
                       WHERE w.session_id=?1 AND a.stderr_audit_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT a.stdout_audit_artifact_id FROM workflow_attempts a
                       JOIN managed_workflows w ON w.id=a.workflow_id
                       WHERE w.session_id=?1 AND a.stdout_audit_artifact_id IS NOT NULL
                       UNION ALL
                       SELECT script_artifact_id FROM result_analysis_runs WHERE session_id=?1
                       UNION ALL
                       SELECT d.artifact_id
                       FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1 AND d.artifact_id IS NOT NULL
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .load::<ArtifactId>(conn)?
                .into_iter()
                .map(|row| row.artifact_id)
                .collect::<Vec<_>>();
                let scope_id = schema::users::table
                    .filter(schema::users::id.eq(&user_id.0))
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                super::artifact::release_refs(
                    conn,
                    scope_id,
                    artifact_ids,
                    crate::application::now_ms(),
                )?;

                diesel::sql_query(
                    "UPDATE workspaces SET active_version_id=NULL
                     WHERE session_id=?1 AND active_version_id IS NOT NULL",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM run_comparisons WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM dataset_views WHERE source_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM dataset_lineage_snapshots WHERE derived_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     ) OR source_dataset_id IN (
                       SELECT d.id FROM result_datasets d
                       LEFT JOIN simulation_runs r ON r.id=d.run_id
                       LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id
                       WHERE COALESCE(r.session_id,a.session_id)=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM result_analysis_inputs WHERE analysis_run_id IN (
                       SELECT id FROM result_analysis_runs WHERE session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM result_datasets WHERE run_id IN (
                       SELECT id FROM simulation_runs WHERE session_id=?1
                     ) OR analysis_run_id IN (
                       SELECT id FROM result_analysis_runs WHERE session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM result_analysis_runs WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM managed_workflows WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM study_configurations WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM simulation_runs WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM messages WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query(
                    "DELETE FROM publish_receipts WHERE version_id IN (
                       SELECT v.id FROM model_versions v
                       JOIN workspaces w ON w.id=v.workspace_id
                       WHERE w.session_id=?1
                     )",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .execute(conn)?;
                diesel::sql_query("DELETE FROM script_resources WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM data_resources WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM upload_batches WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                diesel::sql_query("DELETE FROM workspaces WHERE session_id=?1")
                    .bind::<diesel::sql_types::Integer, _>(session_id)
                    .execute(conn)?;
                let deleted = diesel::update(
                    schema::sessions::table
                        .filter(schema::sessions::id.eq(session_id))
                        .filter(schema::sessions::user_id.eq(user_id.0)),
                )
                .set((
                    schema::sessions::title.eq("已删除会话"),
                    schema::sessions::deletion_status.eq("deleted"),
                    schema::sessions::deleted_at_ms.eq(Some(crate::application::now_ms())),
                ))
                .execute(conn)?;
                debug_assert_eq!(deleted, 1);
                Ok(Vec::new())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn require_live_session(
    conn: &mut SqliteConnection,
    user_id: &UserId,
    session_id: i32,
) -> RepoResult<()> {
    schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::user_id.eq(&user_id.0))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    Ok(())
}

fn session_deletion_impact(
    conn: &mut SqliteConnection,
    session_id: i32,
) -> RepoResult<SessionDeletionImpact> {
    let row = diesel::sql_query(
        r#"
        WITH session_artifacts(artifact_id) AS (
          SELECT d.artifact_id FROM draft_dataframes d JOIN workspace_drafts wd ON wd.id=d.draft_id JOIN workspaces w ON w.id=wd.workspace_id WHERE w.session_id=?1
          UNION SELECT d.artifact_id FROM model_dataframes d JOIN model_versions v ON v.id=d.version_id JOIN workspaces w ON w.id=v.workspace_id WHERE w.session_id=?1
          UNION SELECT c.normalized_artifact_id FROM upload_candidates c JOIN upload_batches b ON b.id=c.batch_id WHERE b.session_id=?1 AND c.normalized_artifact_id IS NOT NULL
          UNION SELECT f.source_artifact_id FROM upload_files f JOIN upload_batches b ON b.id=f.batch_id WHERE b.session_id=?1 AND f.source_artifact_id IS NOT NULL
          UNION SELECT normalized_artifact_id FROM data_resources WHERE session_id=?1 AND normalized_artifact_id IS NOT NULL
          UNION SELECT source_artifact_id FROM data_resources WHERE session_id=?1 AND source_artifact_id IS NOT NULL
          UNION SELECT p.pending_artifact_id FROM data_resource_replacement_previews p JOIN data_resources d ON d.id=p.old_resource_id WHERE d.session_id=?1
          UNION SELECT artifact_id FROM script_resources WHERE session_id=?1
          UNION SELECT script_artifact_id FROM simulation_runs WHERE session_id=?1 AND script_artifact_id IS NOT NULL
          UNION SELECT i.artifact_id FROM workflow_input_snapshots i JOIN managed_workflows w ON w.id=i.workflow_id WHERE w.session_id=?1 AND i.artifact_id IS NOT NULL
          UNION SELECT a.script_artifact_id FROM workflow_attempts a JOIN managed_workflows w ON w.id=a.workflow_id WHERE w.session_id=?1 AND a.script_artifact_id IS NOT NULL
          UNION SELECT a.stderr_audit_artifact_id FROM workflow_attempts a JOIN managed_workflows w ON w.id=a.workflow_id WHERE w.session_id=?1 AND a.stderr_audit_artifact_id IS NOT NULL
          UNION SELECT a.stdout_audit_artifact_id FROM workflow_attempts a JOIN managed_workflows w ON w.id=a.workflow_id WHERE w.session_id=?1 AND a.stdout_audit_artifact_id IS NOT NULL
          UNION SELECT script_artifact_id FROM result_analysis_runs WHERE session_id=?1
          UNION SELECT d.artifact_id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id WHERE COALESCE(r.session_id,a.session_id)=?1 AND d.artifact_id IS NOT NULL
        )
        SELECT
          (SELECT count(*) FROM model_versions v JOIN workspaces w ON w.id=v.workspace_id WHERE w.session_id=?1) AS model_count,
          (SELECT count(*) FROM data_resources WHERE session_id=?1 AND status IN ('live','needs_rename')) AS data_resource_count,
          (SELECT count(*) FROM script_resources WHERE session_id=?1) AS script_count,
          (SELECT count(*) FROM simulation_runs WHERE session_id=?1 AND deleted_at_ms IS NULL) AS run_count,
          (SELECT count(*) FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id WHERE COALESCE(r.session_id,a.session_id)=?1 AND d.deleted_at_ms IS NULL) AS result_count,
          (SELECT count(*) FROM result_analysis_runs WHERE session_id=?1) AS analysis_artifact_count,
          COALESCE((SELECT sum(a.size_bytes) FROM artifacts a JOIN session_artifacts s ON s.artifact_id=a.id),0) AS logical_bytes,
          (SELECT count(*) FROM upload_batches WHERE session_id=?1) AS upload_batch_count,
          (SELECT count(*) FROM data_resource_replacement_previews p JOIN data_resources d ON d.id=p.old_resource_id WHERE d.session_id=?1) AS replacement_preview_count,
          COALESCE((SELECT group_concat(item,'|') FROM (
            SELECT 'model:'||v.id||':'||v.content_hash AS item FROM model_versions v JOIN workspaces w ON w.id=v.workspace_id WHERE w.session_id=?1
            UNION ALL SELECT 'draft:'||wd.id||':'||wd.revision||':'||COALESCE(wd.base_version_id,-1)||':'||wd.updated_at_ms FROM workspace_drafts wd JOIN workspaces w ON w.id=wd.workspace_id WHERE w.session_id=?1
            UNION ALL SELECT 'draft_frame:'||dd.id||':'||dd.table_name||':'||dd.artifact_id||':'||dd.content_hash FROM draft_dataframes dd JOIN workspace_drafts wd ON wd.id=dd.draft_id JOIN workspaces w ON w.id=wd.workspace_id WHERE w.session_id=?1
            UNION ALL SELECT 'resource:'||id||':'||status||':'||schema_hash||':'||COALESCE(normalized_artifact_id,-1)||':'||COALESCE(source_artifact_id,-1)||':'||COALESCE(replaced_by_resource_id,'') FROM data_resources WHERE session_id=?1
            UNION ALL SELECT 'script:'||id||':'||artifact_id||':'||content_hash FROM script_resources WHERE session_id=?1
            UNION ALL SELECT 'run:'||id||':'||status||':'||COALESCE(script_artifact_id,-1)||':'||COALESCE(script_message_id,-1) FROM simulation_runs WHERE session_id=?1
            UNION ALL SELECT 'analysis:'||id||':'||status||':'||script_artifact_id FROM result_analysis_runs WHERE session_id=?1
            UNION ALL SELECT 'result:'||d.id||':'||COALESCE(d.artifact_id,-1)||':'||d.schema_hash||':'||COALESCE(d.deleted_at_ms,-1) FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id WHERE COALESCE(r.session_id,a.session_id)=?1
            UNION ALL SELECT 'upload:'||id||':'||revision||':'||status||':'||model_publish_status||':'||resource_publish_status||':'||reserved_bytes FROM upload_batches WHERE session_id=?1
            UNION ALL SELECT 'candidate:'||c.id||':'||COALESCE(c.normalized_artifact_id,-1)||':'||COALESCE(c.purpose,'')||':'||c.selected||':'||COALESCE(c.schema_hash,'') FROM upload_candidates c JOIN upload_batches b ON b.id=c.batch_id WHERE b.session_id=?1
            UNION ALL SELECT 'file:'||f.id||':'||COALESCE(f.source_artifact_id,-1)||':'||f.sha256 FROM upload_files f JOIN upload_batches b ON b.id=f.batch_id WHERE b.session_id=?1
            UNION ALL SELECT 'preview:'||p.token_hash||':'||p.pending_artifact_id||':'||p.pending_resource_id||':'||p.impact_hash||':'||p.expires_at_ms FROM data_resource_replacement_previews p JOIN data_resources d ON d.id=p.old_resource_id WHERE d.session_id=?1
            ORDER BY item
          )), '') AS impact_fingerprint
        "#,
    )
    .bind::<Integer, _>(session_id)
    .load::<SessionDeletionImpactRow>(conn)?
    .into_iter()
    .next()
    .ok_or_else(|| RepoError::InvalidState("session impact missing".into()))?;
    Ok(SessionDeletionImpact {
        model_count: row.model_count,
        data_resource_count: row.data_resource_count,
        script_count: row.script_count,
        run_count: row.run_count,
        result_count: row.result_count,
        analysis_artifact_count: row.analysis_artifact_count,
        logical_bytes: row.logical_bytes,
        upload_batch_count: row.upload_batch_count,
        replacement_preview_count: row.replacement_preview_count,
        impact_fingerprint: row.impact_fingerprint,
    })
}
