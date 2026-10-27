use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use sha2::{Digest, Sha256};

use crate::auth::UserId;
use crate::db::models::{DraftDataframe, DraftRef, WorkspaceDraft};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(QueryableByName, Debug, Clone)]
pub struct WorkspaceRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub session_id: i32,
    #[diesel(sql_type = Nullable<Integer>)]
    pub active_version_id: Option<i32>,
    #[diesel(sql_type = BigInt)]
    pub created_at_ms: i64,
    #[diesel(sql_type = BigInt)]
    pub updated_at_ms: i64,
}

#[derive(QueryableByName, Debug, Clone)]
pub struct VersionRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub workspace_id: i32,
    #[diesel(sql_type = Text)]
    pub description: String,
    #[diesel(sql_type = Text)]
    pub content_hash: String,
    #[diesel(sql_type = Text)]
    pub source_summary_json: String,
    #[diesel(sql_type = Text)]
    pub engine_contract_version: String,
    #[diesel(sql_type = BigInt)]
    pub created_at_ms: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub deleted_at_ms: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    pub tombstone_json: Option<String>,
}

#[derive(QueryableByName, Debug, Clone)]
pub struct VersionDataframeRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub version_id: i32,
    #[diesel(sql_type = Text)]
    pub table_name: String,
    #[diesel(sql_type = Integer)]
    pub artifact_id: i32,
    #[diesel(sql_type = Text)]
    pub schema_json: String,
    #[diesel(sql_type = BigInt)]
    pub row_count: i64,
    #[diesel(sql_type = Text)]
    pub content_hash: String,
    #[diesel(sql_type = BigInt)]
    pub created_at_ms: i64,
}

#[derive(QueryableByName, Debug, Clone)]
pub struct VersionDataFrameReadRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub version_id: i32,
    #[diesel(sql_type = Text)]
    pub table_name: String,
    #[diesel(sql_type = Integer)]
    pub artifact_id: i32,
    #[diesel(sql_type = Text)]
    pub schema_json: String,
    #[diesel(sql_type = BigInt)]
    pub row_count: i64,
    #[diesel(sql_type = Text)]
    pub content_hash: String,
    #[diesel(sql_type = BigInt)]
    pub size_bytes: i64,
}

#[derive(QueryableByName, Debug, Clone)]
pub struct DraftDataFrameReadRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Integer)]
    pub draft_id: i32,
    #[diesel(sql_type = BigInt)]
    pub revision: i64,
    #[diesel(sql_type = Text)]
    pub table_name: String,
    #[diesel(sql_type = Integer)]
    pub artifact_id: i32,
    #[diesel(sql_type = Text)]
    pub schema_json: String,
    #[diesel(sql_type = BigInt)]
    pub row_count: i64,
    #[diesel(sql_type = Text)]
    pub content_hash: String,
    #[diesel(sql_type = BigInt)]
    pub size_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct DraftSnapshot {
    pub draft: WorkspaceDraft,
    pub dataframes: Vec<DraftDataframe>,
}

#[derive(Debug, Clone)]
pub struct VersionSnapshot {
    pub version: VersionRow,
    pub dataframes: Vec<VersionDataframeRow>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    pub workspace: WorkspaceRow,
    pub active: Option<VersionSnapshot>,
    pub draft: Option<DraftSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PublishResult {
    pub version_id: i32,
    pub deduplicated: bool,
    pub replayed: bool,
    pub release_artifact_ids: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct PublishCommand {
    pub expected: DraftRef,
    pub client_publish_id: String,
    pub description: String,
    pub engine_contract: String,
    pub now_ms: i64,
}

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

#[derive(Debug)]
pub enum VersionRepoError {
    Repository(RepoError),
    DraftConflict,
    DraftRequired,
    DraftPresent,
    ActiveVersion,
    Busy,
    ImpactChanged,
    InvalidToken,
    EmptyDraft,
}

impl From<RepoError> for VersionRepoError {
    fn from(value: RepoError) -> Self {
        Self::Repository(value)
    }
}

impl From<diesel::result::Error> for VersionRepoError {
    fn from(value: diesel::result::Error) -> Self {
        Self::Repository(RepoError::Query(value))
    }
}

#[derive(Clone)]
pub struct VersionRepo {
    pool: DbPool,
}

impl VersionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn description_inputs(
        &self,
        user: &UserId,
        session_id: i32,
        expected: DraftRef,
    ) -> Result<(Vec<(String, String)>, Vec<(String, String)>), VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let workspace = load_workspace_by_session(&mut conn, &user, session_id)?;
            let draft = schema::workspace_drafts::table
                .filter(schema::workspace_drafts::workspace_id.eq(workspace.id))
                .first::<WorkspaceDraft>(&mut conn)
                .optional()?
                .ok_or(VersionRepoError::DraftRequired)?;
            if draft.id != expected.id || draft.revision != expected.revision {
                return Err(VersionRepoError::DraftConflict);
            }
            let current = schema::draft_dataframes::table
                .filter(schema::draft_dataframes::draft_id.eq(draft.id))
                .order(schema::draft_dataframes::table_name.asc())
                .load::<DraftDataframe>(&mut conn)?;
            let previous = match draft.base_version_id {
                Some(version_id) => diesel::sql_query(
                    "SELECT * FROM model_dataframes WHERE version_id=?1 ORDER BY table_name",
                )
                .bind::<Integer, _>(version_id)
                .load::<VersionDataframeRow>(&mut conn)?,
                None => Vec::new(),
            };
            let previous = previous
                .iter()
                .map(|f| (f.table_name.clone(), f.content_hash.clone()))
                .collect::<Vec<_>>();
            let current = current
                .iter()
                .map(|f| (f.table_name.clone(), f.content_hash.clone()))
                .collect::<Vec<_>>();
            Ok((previous, current))
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn find_version_dataframe(
        &self,
        user: &UserId,
        dataframe_id: i32,
    ) -> Result<VersionDataFrameReadRow, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::sql_query(
                "SELECT md.id,md.version_id,md.table_name,md.artifact_id,md.schema_json,md.row_count,md.content_hash,a.size_bytes \
                 FROM model_dataframes md \
                 JOIN model_versions v ON v.id=md.version_id \
                 JOIN workspaces w ON w.id=v.workspace_id \
                 JOIN sessions s ON s.id=w.session_id \
                 JOIN artifacts a ON a.id=md.artifact_id \
                 WHERE md.id=?1 AND s.user_id=?2 AND s.deletion_status='live' AND v.deleted_at_ms IS NULL",
            )
            .bind::<Integer, _>(dataframe_id)
            .bind::<Text, _>(user)
            .get_result::<VersionDataFrameReadRow>(&mut conn)
            .optional()?
            .ok_or(VersionRepoError::Repository(RepoError::NotFound))
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn find_draft_dataframe(
        &self,
        user: &UserId,
        dataframe_id: i32,
        draft_id: i32,
        expected_revision: i64,
    ) -> Result<DraftDataFrameReadRow, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let row = diesel::sql_query(
                "SELECT dd.id,dd.draft_id,wd.revision,dd.table_name,dd.artifact_id,dd.schema_json,dd.row_count,dd.content_hash,a.size_bytes \
                 FROM draft_dataframes dd \
                 JOIN workspace_drafts wd ON wd.id=dd.draft_id \
                 JOIN workspaces w ON w.id=wd.workspace_id \
                 JOIN sessions s ON s.id=w.session_id \
                 JOIN artifacts a ON a.id=dd.artifact_id \
                 WHERE dd.id=?1 AND s.user_id=?2 AND s.deletion_status='live'",
            )
            .bind::<Integer, _>(dataframe_id)
            .bind::<Text, _>(user)
            .get_result::<DraftDataFrameReadRow>(&mut conn)
            .optional()?
            .ok_or(VersionRepoError::Repository(RepoError::NotFound))?;
            if row.draft_id != draft_id || row.revision != expected_revision {
                return Err(VersionRepoError::DraftConflict);
            }
            Ok(row)
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_draft_dataframe(
        &self,
        user: &UserId,
        session_id: i32,
        expected: Option<DraftRef>,
        table_name: String,
        artifact_id: i32,
        schema_json: String,
        row_count: i64,
        content_hash: String,
        now_ms: i64,
    ) -> Result<(DraftRef, Option<i32>), VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, VersionRepoError, _>(|conn| {
                let workspace = load_workspace_by_session(conn, &user, session_id)?;
                ensure_artifacts_owned(conn, &user, std::iter::once(artifact_id))?;
                let current = schema::workspace_drafts::table
                    .filter(schema::workspace_drafts::workspace_id.eq(workspace.id))
                    .first::<WorkspaceDraft>(conn)
                    .optional()?;
                let current_ref = current.as_ref().map(|draft| DraftRef { id: draft.id, revision: draft.revision });
                if current_ref != expected { return Err(VersionRepoError::DraftConflict); }
                let (draft_id, revision) = if let Some(draft) = current {
                    (draft.id, draft.revision + 1)
                } else {
                    diesel::insert_into(schema::workspace_drafts::table).values((
                        schema::workspace_drafts::workspace_id.eq(workspace.id),
                        schema::workspace_drafts::base_version_id.eq(workspace.active_version_id),
                        schema::workspace_drafts::revision.eq(1_i64),
                        schema::workspace_drafts::diagnostics_json.eq("{}"),
                        schema::workspace_drafts::created_at_ms.eq(now_ms),
                        schema::workspace_drafts::updated_at_ms.eq(now_ms),
                    )).execute(conn)?;
                    let draft_id = last_id(conn)?;
                    if let Some(version_id) = workspace.active_version_id {
                        diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) SELECT ?1,table_name,artifact_id,schema_json,row_count,content_hash,?2 FROM model_dataframes WHERE version_id=?3")
                            .bind::<Integer,_>(draft_id).bind::<BigInt,_>(now_ms).bind::<Integer,_>(version_id).execute(conn)?;
                        diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+(SELECT count(*) FROM model_dataframes WHERE version_id=?2 AND artifact_id=storage_scope_artifacts.artifact_id) WHERE storage_scope_id=(SELECT storage_scope_id FROM users WHERE id=?1) AND artifact_id IN(SELECT artifact_id FROM model_dataframes WHERE version_id=?2)")
                            .bind::<Text,_>(&user).bind::<Integer,_>(version_id).execute(conn)?;
                    }
                    (draft_id, 1)
                };
                let replaced = schema::draft_dataframes::table
                    .filter(schema::draft_dataframes::draft_id.eq(draft_id))
                    .filter(schema::draft_dataframes::table_name.eq(&table_name))
                    .select(schema::draft_dataframes::artifact_id)
                    .first::<i32>(conn).optional()?;
                diesel::delete(schema::draft_dataframes::table
                    .filter(schema::draft_dataframes::draft_id.eq(draft_id))
                    .filter(schema::draft_dataframes::table_name.eq(&table_name)))
                    .execute(conn)?;
                diesel::insert_into(schema::draft_dataframes::table).values(crate::db::models::NewDraftDataframe {
                    draft_id, table_name, artifact_id, schema_json, row_count, content_hash, created_at_ms: now_ms,
                }).execute(conn)?;
                diesel::update(schema::workspace_drafts::table.find(draft_id)).set((
                    schema::workspace_drafts::revision.eq(revision),
                    schema::workspace_drafts::updated_at_ms.eq(now_ms),
                )).execute(conn)?;
                Ok((DraftRef { id: draft_id, revision }, replaced))
            })
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn workspace(
        &self,
        user: &UserId,
        session_id: i32,
        now_ms: i64,
    ) -> Result<WorkspaceSnapshot, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceSnapshot, VersionRepoError, _>(|conn| {
                ensure_session(conn, &user, session_id)?;
                diesel::sql_query("INSERT INTO workspaces(session_id,created_at_ms,updated_at_ms) VALUES(?1,?2,?2) ON CONFLICT(session_id) DO NOTHING")
                    .bind::<Integer,_>(session_id).bind::<BigInt,_>(now_ms).execute(conn)?;
                let workspace = load_workspace_by_session(conn, &user, session_id)?;
                load_workspace_snapshot(conn, workspace)
            })
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn list_versions(
        &self,
        user: &UserId,
        session_id: i32,
    ) -> Result<Vec<VersionSnapshot>, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let workspace = load_workspace_by_session(&mut conn, &user, session_id)?;
            let versions = diesel::sql_query("SELECT v.* FROM model_versions v WHERE v.workspace_id=?1 AND v.deleted_at_ms IS NULL ORDER BY v.created_at_ms DESC,v.id DESC")
                .bind::<Integer,_>(workspace.id).load::<VersionRow>(&mut conn)?;
            versions.into_iter().map(|version| load_version_snapshot(&mut conn, version)).collect()
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn find_version(
        &self,
        user: &UserId,
        version_id: i32,
    ) -> Result<VersionSnapshot, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let version = scoped_version(&mut conn, &user, version_id, true)?;
            load_version_snapshot(&mut conn, version)
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn find_publish_receipt(
        &self,
        user: &UserId,
        session_id: i32,
        expected: DraftRef,
        client_publish_id: &str,
    ) -> Result<Option<i32>, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let client_publish_id = client_publish_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let workspace = load_workspace_by_session(&mut conn, &user, session_id)?;
            #[derive(QueryableByName)]
            struct Receipt {
                #[diesel(sql_type = Integer)]
                version_id: i32,
            }
            diesel::sql_query(
                "SELECT r.version_id FROM publish_receipts r JOIN model_versions v ON v.id=r.version_id WHERE r.draft_id=?1 AND r.draft_revision=?2 AND r.client_publish_id=?3 AND v.workspace_id=?4 AND v.deleted_at_ms IS NULL",
            )
            .bind::<Integer, _>(expected.id)
            .bind::<BigInt, _>(expected.revision)
            .bind::<Text, _>(client_publish_id)
            .bind::<Integer, _>(workspace.id)
            .get_result::<Receipt>(&mut conn)
            .optional()
            .map(|receipt| receipt.map(|receipt| receipt.version_id))
            .map_err(Into::into)
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn publish(
        &self,
        user: &UserId,
        session_id: i32,
        command: PublishCommand,
    ) -> Result<PublishResult, VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let PublishCommand {
            expected,
            client_publish_id,
            description,
            engine_contract,
            now_ms,
        } = command;
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<PublishResult, VersionRepoError, _>(|conn| {
                let workspace = load_workspace_by_session(conn, &user, session_id)?;
                #[derive(QueryableByName)] struct Receipt { #[diesel(sql_type=Integer)] version_id:i32 }
                if let Some(receipt) = diesel::sql_query("SELECT version_id FROM publish_receipts WHERE draft_id=?1 AND draft_revision=?2 AND client_publish_id=?3")
                    .bind::<Integer,_>(expected.id).bind::<BigInt,_>(expected.revision).bind::<Text,_>(&client_publish_id)
                    .get_result::<Receipt>(conn).optional()? {
                    let version = scoped_version(conn, &user, receipt.version_id, true)?;
                    if version.workspace_id != workspace.id { return Err(VersionRepoError::Repository(RepoError::NotFound)); }
                    return Ok(PublishResult { version_id: receipt.version_id, deduplicated: false, replayed: true, release_artifact_ids: vec![] });
                }
                let draft = schema::workspace_drafts::table.filter(schema::workspace_drafts::workspace_id.eq(workspace.id)).first::<WorkspaceDraft>(conn).optional()?;
                let Some(draft) = draft else { return Err(VersionRepoError::DraftRequired); };
                if draft.id != expected.id || draft.revision != expected.revision { return Err(VersionRepoError::DraftConflict); }
                let frames = schema::draft_dataframes::table.filter(schema::draft_dataframes::draft_id.eq(draft.id)).order(schema::draft_dataframes::table_name.asc()).load::<DraftDataframe>(conn)?;
                if frames.is_empty() { return Err(VersionRepoError::EmptyDraft); }
                ensure_artifacts_owned(conn, &user, frames.iter().map(|frame| frame.artifact_id))?;
                let content_hash = content_hash(&frames);
                let existing = diesel::sql_query("SELECT * FROM model_versions WHERE workspace_id=?1 AND content_hash=?2 AND deleted_at_ms IS NULL LIMIT 1")
                    .bind::<Integer,_>(workspace.id).bind::<Text,_>(&content_hash).get_result::<VersionRow>(conn).optional()?;
                let (version_id, deduplicated, release_artifact_ids) = if let Some(version) = existing {
                    (version.id, true, frames.iter().map(|frame| frame.artifact_id).collect())
                } else {
                    let summary = serde_json::json!({"draftId":draft.id,"draftRevision":draft.revision,"tables":frames.iter().map(|frame| &frame.table_name).collect::<Vec<_>>()}).to_string();
                    diesel::sql_query("INSERT INTO model_versions(workspace_id,description,content_hash,source_summary_json,engine_contract_version,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6)")
                        .bind::<Integer,_>(workspace.id).bind::<Text,_>(&description).bind::<Text,_>(&content_hash).bind::<Text,_>(summary).bind::<Text,_>(&engine_contract).bind::<BigInt,_>(now_ms).execute(conn)?;
                    let version_id = last_id(conn)?;
                    for frame in &frames {
                        diesel::sql_query("INSERT INTO model_dataframes(version_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                            .bind::<Integer,_>(version_id).bind::<Text,_>(&frame.table_name).bind::<Integer,_>(frame.artifact_id).bind::<Text,_>(&frame.schema_json).bind::<BigInt,_>(frame.row_count).bind::<Text,_>(&frame.content_hash).bind::<BigInt,_>(now_ms).execute(conn)?;
                    }
                    (version_id, false, vec![])
                };
                diesel::sql_query("UPDATE workspaces SET active_version_id=?1,updated_at_ms=?2 WHERE id=?3")
                    .bind::<Integer,_>(version_id).bind::<BigInt,_>(now_ms).bind::<Integer,_>(workspace.id).execute(conn)?;
                diesel::delete(schema::workspace_drafts::table.find(draft.id)).execute(conn)?;
                diesel::sql_query("INSERT INTO publish_receipts(draft_id,draft_revision,client_publish_id,version_id,created_at_ms) VALUES(?1,?2,?3,?4,?5)")
                    .bind::<Integer,_>(draft.id).bind::<BigInt,_>(draft.revision).bind::<Text,_>(client_publish_id).bind::<Integer,_>(version_id).bind::<BigInt,_>(now_ms).execute(conn)?;
                Ok(PublishResult { version_id, deduplicated, replayed: false, release_artifact_ids })
            })
        }).await.map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn activate(
        &self,
        user: &UserId,
        version_id: i32,
        discard_draft: bool,
        expected: Option<DraftRef>,
        now_ms: i64,
    ) -> Result<(WorkspaceSnapshot, Vec<i32>), VersionRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(WorkspaceSnapshot, Vec<i32>), VersionRepoError, _>(
                |conn| {
                    let version = scoped_version(conn, &user, version_id, true)?;
                    let workspace = load_workspace_by_id(conn, &user, version.workspace_id)?;
                    let draft = schema::workspace_drafts::table
                        .filter(schema::workspace_drafts::workspace_id.eq(workspace.id))
                        .first::<WorkspaceDraft>(conn)
                        .optional()?;
                    let mut release = vec![];
                    match draft {
                        Some(_) if !discard_draft => return Err(VersionRepoError::DraftPresent),
                        Some(draft) => {
                            if expected
                                != Some(DraftRef {
                                    id: draft.id,
                                    revision: draft.revision,
                                })
                            {
                                return Err(VersionRepoError::DraftConflict);
                            }
                            release = schema::draft_dataframes::table
                                .filter(schema::draft_dataframes::draft_id.eq(draft.id))
                                .select(schema::draft_dataframes::artifact_id)
                                .load(conn)?;
                            diesel::delete(schema::workspace_drafts::table.find(draft.id))
                                .execute(conn)?;
                        }
                        None if expected.is_some() => return Err(VersionRepoError::DraftConflict),
                        None => {}
                    }
                    diesel::sql_query(
                        "UPDATE workspaces SET active_version_id=?1,updated_at_ms=?2 WHERE id=?3",
                    )
                    .bind::<Integer, _>(version_id)
                    .bind::<BigInt, _>(now_ms)
                    .bind::<Integer, _>(workspace.id)
                    .execute(conn)?;
                    let workspace = load_workspace_by_id(conn, &user, workspace.id)?;
                    let snapshot = load_workspace_snapshot(conn, workspace)?;
                    Ok((snapshot, release))
                },
            )
        })
        .await
        .map_err(|error| VersionRepoError::Repository(RepoError::Join(error)))?
    }

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

fn ensure_session(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
) -> Result<(), VersionRepoError> {
    let found = schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?;
    found
        .map(|_| ())
        .ok_or(VersionRepoError::Repository(RepoError::NotFound))
}

fn load_workspace_by_session(
    conn: &mut SqliteConnection,
    user: &str,
    session_id: i32,
) -> Result<WorkspaceRow, VersionRepoError> {
    diesel::sql_query("SELECT w.id,w.session_id,w.active_version_id,w.created_at_ms,w.updated_at_ms FROM workspaces w JOIN sessions s ON s.id=w.session_id WHERE w.session_id=?1 AND s.user_id=?2 AND s.deletion_status='live'")
        .bind::<Integer,_>(session_id).bind::<Text,_>(user).get_result(conn).optional()?.ok_or(VersionRepoError::Repository(RepoError::NotFound))
}

fn load_workspace_by_id(
    conn: &mut SqliteConnection,
    user: &str,
    workspace_id: i32,
) -> Result<WorkspaceRow, VersionRepoError> {
    diesel::sql_query("SELECT w.id,w.session_id,w.active_version_id,w.created_at_ms,w.updated_at_ms FROM workspaces w JOIN sessions s ON s.id=w.session_id WHERE w.id=?1 AND s.user_id=?2 AND s.deletion_status='live'")
        .bind::<Integer,_>(workspace_id).bind::<Text,_>(user).get_result(conn).optional()?.ok_or(VersionRepoError::Repository(RepoError::NotFound))
}

fn scoped_version(
    conn: &mut SqliteConnection,
    user: &str,
    version_id: i32,
    live: bool,
) -> Result<VersionRow, VersionRepoError> {
    let suffix = if live {
        " AND v.deleted_at_ms IS NULL"
    } else {
        ""
    };
    diesel::sql_query(format!("SELECT v.* FROM model_versions v JOIN workspaces w ON w.id=v.workspace_id JOIN sessions s ON s.id=w.session_id WHERE v.id=?1 AND s.user_id=?2 AND s.deletion_status='live'{suffix}"))
        .bind::<Integer,_>(version_id).bind::<Text,_>(user).get_result(conn).optional()?.ok_or(VersionRepoError::Repository(RepoError::NotFound))
}

fn load_version_snapshot(
    conn: &mut SqliteConnection,
    version: VersionRow,
) -> Result<VersionSnapshot, VersionRepoError> {
    let dataframes =
        diesel::sql_query("SELECT * FROM model_dataframes WHERE version_id=?1 ORDER BY table_name")
            .bind::<Integer, _>(version.id)
            .load(conn)?;
    Ok(VersionSnapshot {
        version,
        dataframes,
    })
}

fn load_workspace_snapshot(
    conn: &mut SqliteConnection,
    workspace: WorkspaceRow,
) -> Result<WorkspaceSnapshot, VersionRepoError> {
    // Active ownership was already checked through the workspace; avoid a second user predicate.
    let active = match workspace.active_version_id {
        Some(id) => {
            diesel::sql_query("SELECT * FROM model_versions WHERE id=?1 AND deleted_at_ms IS NULL")
                .bind::<Integer, _>(id)
                .get_result::<VersionRow>(conn)
                .optional()?
                .map(|row| load_version_snapshot(conn, row))
                .transpose()?
        }
        None => None,
    };
    let draft = schema::workspace_drafts::table
        .filter(schema::workspace_drafts::workspace_id.eq(workspace.id))
        .first::<WorkspaceDraft>(conn)
        .optional()?
        .map(|draft| {
            let dataframes = schema::draft_dataframes::table
                .filter(schema::draft_dataframes::draft_id.eq(draft.id))
                .order(schema::draft_dataframes::table_name.asc())
                .load(conn)?;
            Ok::<_, VersionRepoError>(DraftSnapshot { draft, dataframes })
        })
        .transpose()?;
    Ok(WorkspaceSnapshot {
        workspace,
        active,
        draft,
    })
}

fn ensure_artifacts_owned(
    conn: &mut SqliteConnection,
    user: &str,
    ids: impl Iterator<Item = i32>,
) -> Result<(), VersionRepoError> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type=BigInt)]
        count: i64,
    }
    for id in ids {
        let count = diesel::sql_query("SELECT count(*) count FROM storage_scope_artifacts ssa JOIN users u ON u.storage_scope_id=ssa.storage_scope_id WHERE u.id=?1 AND ssa.artifact_id=?2")
            .bind::<Text,_>(user).bind::<Integer,_>(id).get_result::<Count>(conn)?.count;
        if count != 1 {
            return Err(VersionRepoError::Repository(RepoError::NotFound));
        }
    }
    Ok(())
}

fn content_hash(frames: &[DraftDataframe]) -> String {
    let mut hasher = Sha256::new();
    for frame in frames {
        hasher.update(frame.table_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(frame.content_hash.as_bytes());
        hasher.update(b"\0");
        let schema = serde_json::from_str(&frame.schema_json)
            .map(|value| canonical_json(&value))
            .unwrap_or_else(|_| frame.schema_json.clone());
        hasher.update(schema.as_bytes());
        hasher.update(b"\0");
        hasher.update(frame.row_count.to_string().as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).unwrap(),
    }
}

fn last_id(conn: &mut SqliteConnection) -> RepoResult<i32> {
    diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()"))
        .get_result(conn)
        .map_err(RepoError::from)
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
