use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{
    DraftRef, ModelImport, ModelImportCandidate, NewDraftDataframe, NewModelImport,
    NewModelImportCandidate, WorkspaceDraft,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct ModelImportRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct ModelImportRecord {
    pub import: ModelImport,
    pub candidates: Vec<ModelImportCandidate>,
}

impl ModelImportRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn ensure_workspace(
        &self,
        user_id: &UserId,
        session_id: i32,
        now_ms: i64,
    ) -> RepoResult<i32> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
                let owns = schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(user_id))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?
                    .is_some();
                if !owns {
                    return Err(RepoError::NotFound);
                }
                diesel::sql_query(
                    "INSERT INTO workspaces(session_id,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,?2) ON CONFLICT(session_id) DO NOTHING",
                )
                .bind::<diesel::sql_types::Integer, _>(session_id)
                .bind::<diesel::sql_types::BigInt, _>(now_ms)
                .execute(conn)?;
                schema::workspaces::table
                    .filter(schema::workspaces::session_id.eq(session_id))
                    .select(schema::workspaces::id)
                    .first(conn)
                    .map_err(RepoError::from)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn current_draft(&self, workspace_id: i32) -> RepoResult<Option<DraftRef>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::workspace_drafts::table
                .filter(schema::workspace_drafts::workspace_id.eq(workspace_id))
                .select((
                    schema::workspace_drafts::id,
                    schema::workspace_drafts::revision,
                ))
                .first::<(i32, i64)>(&mut conn)
                .optional()
                .map(|value| value.map(|(id, revision)| DraftRef { id, revision }))
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn create(
        &self,
        record: NewModelImport,
        candidates: Vec<NewModelImportCandidate>,
    ) -> RepoResult<ModelImportRecord> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
                diesel::insert_into(schema::model_imports::table)
                    .values(&record)
                    .execute(conn)?;
                let import = schema::model_imports::table
                    .order(schema::model_imports::id.desc())
                    .first::<ModelImport>(conn)?;
                let candidates = candidates
                    .into_iter()
                    .map(|mut candidate| {
                        candidate.import_id = import.id;
                        candidate
                    })
                    .collect::<Vec<_>>();
                diesel::insert_into(schema::model_import_candidates::table)
                    .values(&candidates)
                    .execute(conn)?;
                let candidates = schema::model_import_candidates::table
                    .filter(schema::model_import_candidates::import_id.eq(import.id))
                    .order(schema::model_import_candidates::id.asc())
                    .load(conn)?;
                Ok(ModelImportRecord { import, candidates })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user_id: &UserId, id: i32) -> RepoResult<ModelImportRecord> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let import = schema::model_imports::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::model_imports::session_id)),
                )
                .filter(schema::model_imports::id.eq(id))
                .filter(schema::sessions::user_id.eq(user_id))
                .select(schema::model_imports::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let candidates = schema::model_import_candidates::table
                .filter(schema::model_import_candidates::import_id.eq(id))
                .order(schema::model_import_candidates::id.asc())
                .load(&mut conn)?;
            Ok(ModelImportRecord { import, candidates })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_by_client_id(
        &self,
        user_id: &UserId,
        session_id: i32,
        client_import_id: &str,
    ) -> RepoResult<Option<ModelImportRecord>> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        let client_import_id = client_import_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let import = schema::model_imports::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::model_imports::session_id)),
                )
                .filter(schema::model_imports::session_id.eq(session_id))
                .filter(schema::model_imports::client_import_id.eq(client_import_id))
                .filter(schema::sessions::user_id.eq(user_id))
                .select(schema::model_imports::all_columns)
                .first::<ModelImport>(&mut conn)
                .optional()?;
            let Some(import) = import else {
                return Ok(None);
            };
            let candidates = schema::model_import_candidates::table
                .filter(schema::model_import_candidates::import_id.eq(import.id))
                .order(schema::model_import_candidates::id.asc())
                .load(&mut conn)?;
            Ok(Some(ModelImportRecord { import, candidates }))
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_status(
        &self,
        id: i32,
        status: &str,
        stages_json: String,
        diagnostics_json: String,
        error: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::update(schema::model_imports::table.find(id))
                .set((
                    schema::model_imports::status.eq(status),
                    schema::model_imports::stages_json.eq(stages_json),
                    schema::model_imports::diagnostics_json.eq(diagnostics_json),
                    schema::model_imports::error_code
                        .eq(error.as_ref().map(|(code, _)| code.clone())),
                    schema::model_imports::error_message.eq(error.map(|(_, message)| message)),
                    schema::model_imports::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn replace_candidates(
        &self,
        import_id: i32,
        candidates: Vec<NewModelImportCandidate>,
    ) -> RepoResult<Vec<i32>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
                let old = schema::model_import_candidates::table
                    .filter(schema::model_import_candidates::import_id.eq(import_id))
                    .select(schema::model_import_candidates::artifact_id)
                    .load::<Option<i32>>(conn)?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                diesel::delete(
                    schema::model_import_candidates::table
                        .filter(schema::model_import_candidates::import_id.eq(import_id)),
                )
                .execute(conn)?;
                let candidates = candidates
                    .into_iter()
                    .map(|mut candidate| {
                        candidate.import_id = import_id;
                        candidate
                    })
                    .collect::<Vec<_>>();
                diesel::insert_into(schema::model_import_candidates::table)
                    .values(candidates)
                    .execute(conn)?;
                Ok(old)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn apply_candidates(
        &self,
        user_id: &UserId,
        import_id: i32,
        expected: Option<DraftRef>,
        candidate_ids: Vec<String>,
        now_ms: i64,
    ) -> RepoResult<(DraftRef, Vec<i32>)> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let import = schema::model_imports::table
                    .inner_join(
                        schema::sessions::table
                            .on(schema::sessions::id.eq(schema::model_imports::session_id)),
                    )
                    .filter(schema::model_imports::id.eq(import_id))
                    .filter(schema::sessions::user_id.eq(user_id))
                    .select(schema::model_imports::all_columns)
                    .first::<ModelImport>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if !matches!(
                    import.status.as_str(),
                    "ready_to_apply" | "awaiting_resolution"
                ) {
                    return Err(RepoError::InvalidState(import.status));
                }
                let actual = schema::workspace_drafts::table
                    .filter(schema::workspace_drafts::workspace_id.eq(import.workspace_id))
                    .first::<WorkspaceDraft>(conn)
                    .optional()?
                    .map(|draft| DraftRef {
                        id: draft.id,
                        revision: draft.revision,
                    });
                if actual != expected {
                    return Err(RepoError::VersionConflict);
                }
                let candidates = schema::model_import_candidates::table
                    .filter(schema::model_import_candidates::import_id.eq(import_id))
                    .filter(schema::model_import_candidates::candidate_id.eq_any(&candidate_ids))
                    .load::<ModelImportCandidate>(conn)?;
                if candidates.len() != candidate_ids.len() {
                    return Err(RepoError::InvalidState("模型导入候选已变化".into()));
                }
                let mut table_names = std::collections::HashSet::new();
                if candidates.iter().any(|candidate| {
                    candidate.candidate_kind != "table"
                        || candidate.artifact_id.is_none()
                        || candidate.schema_json.is_none()
                        || candidate.row_count.is_none()
                        || candidate.content_hash.is_none()
                        || candidate
                            .table_name
                            .as_ref()
                            .is_none_or(|table| !table_names.insert(table.clone()))
                }) {
                    return Err(RepoError::InvalidState(
                        "模型导入仍需表映射或去重选择".into(),
                    ));
                }
                let (draft_id, revision) = match actual {
                    Some(draft) => (draft.id, draft.revision + 1),
                    None => {
                        diesel::insert_into(schema::workspace_drafts::table)
                            .values((
                                schema::workspace_drafts::workspace_id.eq(import.workspace_id),
                                schema::workspace_drafts::base_version_id.eq::<Option<i32>>(None),
                                schema::workspace_drafts::revision.eq(1_i64),
                                schema::workspace_drafts::diagnostics_json.eq("{}"),
                                schema::workspace_drafts::created_at_ms.eq(now_ms),
                                schema::workspace_drafts::updated_at_ms.eq(now_ms),
                            ))
                            .execute(conn)?;
                        (
                            schema::workspace_drafts::table
                                .filter(
                                    schema::workspace_drafts::workspace_id.eq(import.workspace_id),
                                )
                                .select(schema::workspace_drafts::id)
                                .first(conn)?,
                            1,
                        )
                    }
                };
                let mut replaced = Vec::new();
                for candidate in candidates {
                    let table_name = candidate.table_name.unwrap();
                    if let Some(artifact_id) = schema::draft_dataframes::table
                        .filter(schema::draft_dataframes::draft_id.eq(draft_id))
                        .filter(schema::draft_dataframes::table_name.eq(&table_name))
                        .select(schema::draft_dataframes::artifact_id)
                        .first::<i32>(conn)
                        .optional()?
                    {
                        replaced.push(artifact_id);
                    }
                    diesel::delete(
                        schema::draft_dataframes::table
                            .filter(schema::draft_dataframes::draft_id.eq(draft_id))
                            .filter(schema::draft_dataframes::table_name.eq(&table_name)),
                    )
                    .execute(conn)?;
                    diesel::insert_into(schema::draft_dataframes::table)
                        .values(NewDraftDataframe {
                            draft_id,
                            table_name,
                            artifact_id: candidate.artifact_id.unwrap(),
                            schema_json: candidate.schema_json.unwrap(),
                            row_count: candidate.row_count.unwrap(),
                            content_hash: candidate.content_hash.unwrap(),
                            created_at_ms: now_ms,
                        })
                        .execute(conn)?;
                }
                diesel::update(schema::workspace_drafts::table.find(draft_id))
                    .set((
                        schema::workspace_drafts::revision.eq(revision),
                        schema::workspace_drafts::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                diesel::update(schema::model_imports::table.find(import_id))
                    .set((
                        schema::model_imports::status.eq("applied"),
                        schema::model_imports::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok((
                    DraftRef {
                        id: draft_id,
                        revision,
                    },
                    replaced,
                ))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}
