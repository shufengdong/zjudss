use std::collections::{BTreeMap, BTreeSet};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use crate::auth::UserId;
use crate::db::models::{
    DataResourceRecord, DataResourceReplacementPreviewRecord, DraftRef, UploadBatchRecord,
    UploadCandidateRecord, UploadFileRecord,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct ResourcePageRow {
    pub resource: DataResourceRecord,
    pub normalized_sha256: String,
    pub normalized_size_bytes: i64,
    pub source_sha256: Option<String>,
    pub source_size_bytes: Option<i64>,
}

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

#[derive(Debug, Clone)]
pub enum StructuredPreviewConfirmation {
    Published(Box<DataResourceRecord>),
    Cancelled,
    Rejected { code: &'static str },
}

#[derive(Debug, Clone)]
pub struct DeletionImpactRow {
    pub resource: DataResourceRecord,
    pub normalized_size_bytes: i64,
    pub pending_configuration_count: i64,
    pub frozen_reference_count: i64,
    pub historical_reference_count: i64,
}

#[derive(Debug, Clone)]
pub struct DeletedResource {
    pub resource: DataResourceRecord,
}

#[derive(Debug, Clone)]
pub struct StructuredPreviewInput {
    pub token_hash: String,
    pub user_id: String,
    pub old_resource_id: String,
    pub pending_artifact_id: i32,
    pub pending_resource_id: String,
    pub expected_content_hash: String,
    pub schema_ref: String,
    pub schema_json: String,
    pub schema_hash: String,
    pub values_json: String,
    pub display_name: String,
    pub proposed_alias: String,
    pub impact_hash: String,
    pub impact_json: String,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

pub struct PromoteDatasetInput {
    pub session_id: i32,
    pub dataset_id: i32,
    pub resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub normalized_alias: String,
    pub now_ms: i64,
}

pub struct DeletionPreviewInput {
    pub resource_id: String,
    pub token_hash: String,
    pub impact_hash: String,
    pub impact_json: String,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Clone)]
pub struct DataResourceRepo {
    pool: DbPool,
}

impl DataResourceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn claim_confirmation(
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
            conn.immediate_transaction(|conn| {
                let batch = owned_batch(conn, &user, batch_id)?;
                if !matches!(
                    batch.status.as_str(),
                    "ready_to_confirm" | "partially_confirmed" | "confirmed"
                ) {
                    return Err(RepoError::InvalidState("upload_state_invalid".into()));
                }
                if let Some(existing) = batch.confirm_operation_id {
                    if existing != operation_id {
                        return Err(RepoError::InvalidState("idempotency_conflict".into()));
                    }
                    return Ok(());
                }
                diesel::update(schema::upload_batches::table.find(batch_id))
                    .set((
                        schema::upload_batches::confirm_operation_id.eq(Some(operation_id)),
                        schema::upload_batches::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(
        &self,
        user: &UserId,
        session_id: i32,
        before: Option<(i64, String)>,
        limit: i64,
    ) -> RepoResult<Vec<ResourcePageRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &user, session_id)?;
            let mut query = schema::data_resources::table
                .filter(schema::data_resources::session_id.eq(session_id))
                .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                .into_boxed();
            if let Some((created, id)) = before {
                query = query.filter(
                    schema::data_resources::created_at_ms.lt(created).or(
                        schema::data_resources::created_at_ms
                            .eq(created)
                            .and(schema::data_resources::id.lt(id)),
                    ),
                );
            }
            let rows = query
                .order((
                    schema::data_resources::created_at_ms.desc(),
                    schema::data_resources::id.desc(),
                ))
                .limit(limit)
                .load::<DataResourceRecord>(&mut conn)?;
            rows.into_iter()
                .map(|resource| enrich_resource(&mut conn, resource))
                .collect()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_current_for_user(
        &self,
        user: &UserId,
    ) -> RepoResult<Vec<DataResourceRecord>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::data_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::data_resources::session_id)),
                )
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                .select(schema::data_resources::all_columns)
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(
        &self,
        user: &UserId,
        id: &str,
        include_historical: bool,
    ) -> RepoResult<ResourcePageRow> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let resource = owned_resource(&mut conn, &user, &id, include_historical)?;
            enrich_resource(&mut conn, resource)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_record(&self, user: &UserId, id: &str) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_resource(&mut conn, &user, &id, true)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn for_batch(
        &self,
        user: &UserId,
        batch_id: i32,
    ) -> RepoResult<Vec<ResourcePageRow>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_batch(&mut conn, &user, batch_id)?;
            let rows = schema::data_resources::table
                .filter(schema::data_resources::source_batch_id.eq(batch_id))
                .filter(schema::data_resources::status.ne("deleted"))
                .order(schema::data_resources::created_at_ms.asc())
                .load::<DataResourceRecord>(&mut conn)?;
            rows.into_iter()
                .map(|row| enrich_resource(&mut conn, row))
                .collect()
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
                let batch = owned_batch(conn, &user, batch_id)?;
                if batch.resource_publish_status == "published" {
                    let rows = schema::data_resources::table.filter(schema::data_resources::source_batch_id.eq(batch_id)).load(conn)?;
                    return Ok(PublishChildResult { resources: rows, replaced_artifact_ids: vec![] });
                }
                if !matches!(batch.status.as_str(), "ready_to_confirm" | "partially_confirmed") {
                    return Err(RepoError::InvalidState("上传批次尚未准备确认".into()));
                }
                let candidates = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(schema::upload_candidates::selected.eq(true))
                    .filter(schema::upload_candidates::purpose.eq("data_resource"))
                    .load::<UploadCandidateRecord>(conn)?;
                if candidates.len() != inputs.len() || inputs.iter().any(|input| !candidates.iter().any(|c| c.id == input.candidate_id)) {
                    return Err(RepoError::InvalidState("资源候选决定不完整".into()));
                }
                let files = schema::upload_files::table.filter(schema::upload_files::batch_id.eq(batch_id)).load::<UploadFileRecord>(conn)?;
                let mut replaced_artifact_ids = Vec::new();
                let mut old_replacement = None;
                let mut replacement_lineage = None;
                if let Some((old_id, action)) = &replacement {
                    if matches!(action.as_str(), "replace" | "save_as_new") {
                        let old = schema::data_resources::table
                            .inner_join(schema::sessions::table.on(schema::sessions::id.eq(schema::data_resources::session_id)))
                            .filter(schema::data_resources::id.eq(old_id))
                            .filter(schema::sessions::user_id.eq(&user))
                            .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                            .select(schema::data_resources::all_columns)
                            .first::<DataResourceRecord>(conn).optional()?.ok_or(RepoError::NotFound)?;
                        let old_content_hash = old
                            .normalized_artifact_id
                            .map(|artifact_id| {
                                schema::artifacts::table
                                    .find(artifact_id)
                                    .select(schema::artifacts::sha256)
                                    .first::<String>(conn)
                            })
                            .transpose()?
                            .unwrap_or_default();
                        replacement_lineage = Some(serde_json::json!({
                            "action": action,
                            "sourceResourceId": old.id.clone(),
                            "replacedResourceId": (action == "replace").then(|| old.id.clone()),
                            "previousContentHash": old_content_hash,
                            "previousSchemaHash": old.schema_hash.clone(),
                        }));
                        if action == "replace" {
                            old_replacement = Some(old);
                        }
                    } else if action == "cancel" {
                        return Ok(PublishChildResult { resources: vec![], replaced_artifact_ids: vec![] });
                    }
                }
                if old_replacement.is_some() && inputs.len() != 1 {
                    return Err(RepoError::InvalidState("替换批次必须只发布一个资源".into()));
                }
                let mut resources = Vec::new();
                let mut source_use_counts = BTreeMap::<i32, (i64, BTreeSet<String>)>::new();
                for input in inputs {
                    let candidate = candidates.iter().find(|candidate| candidate.id == input.candidate_id).ok_or(RepoError::NotFound)?;
                    let artifact_id = candidate.normalized_artifact_id.ok_or_else(|| RepoError::InvalidState("候选缺少规范化 artifact".into()))?;
                    let schema_json = candidate.schema_json.clone().ok_or_else(|| RepoError::InvalidState("候选缺少 schema".into()))?;
                    let schema_hash = candidate.schema_hash.clone().ok_or_else(|| RepoError::InvalidState("候选缺少 schema hash".into()))?;
                    let row_count = candidate.row_count.ok_or_else(|| RepoError::InvalidState("候选缺少行数".into()))?;
                    let source_artifact_id = files.iter().find(|file| file.id == candidate.file_id).and_then(|file| file.source_artifact_id);
                    if let Some(source_artifact_id) = source_artifact_id {
                        let usage = source_use_counts.entry(source_artifact_id).or_default();
                        usage.0 += 1;
                        usage.1.insert(candidate.file_id.clone());
                    }
                    let mut alias = input.alias;
                    let mut normalized_alias = input.normalized_alias;
                    let mut lineage = serde_json::from_str::<serde_json::Value>(&input.lineage_json)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    if let (Some(object), Some(replacement)) =
                        (lineage.as_object_mut(), replacement_lineage.as_ref())
                    {
                        object.insert("replacement".into(), replacement.clone());
                    }
                    let lineage_json = lineage.to_string();
                    if let Some(old) = old_replacement.take() {
                        alias = old.alias.clone();
                        normalized_alias = old.normalized_alias.clone();
                        let backup_alias = format!("data_{}", &old.id[old.id.len().saturating_sub(8)..]);
                        diesel::update(schema::data_resources::table.find(&old.id)).set((
                            schema::data_resources::alias.eq(&backup_alias),
                            schema::data_resources::normalized_alias.eq(backup_alias.to_ascii_lowercase()),
                            schema::data_resources::status.eq("replaced"),
                            schema::data_resources::replaced_by_resource_id.eq(Option::<String>::None),
                            schema::data_resources::updated_at_ms.eq(now_ms),
                        )).execute(conn)?;
                        diesel::sql_query("UPDATE study_configurations SET status='invalidated',updated_at_ms=?1 WHERE id IN(SELECT configuration_id FROM study_input_bindings WHERE resource_id=?2) AND status IN ('choosing_facade','collecting_inputs','ready_to_confirm')")
                            .bind::<BigInt,_>(now_ms)
                            .bind::<Text,_>(&old.id)
                            .execute(conn)?;
                        if let Some(artifact_id) = old.normalized_artifact_id {
                            replaced_artifact_ids.push(artifact_id);
                        }
                    } else {
                        let conflict = schema::data_resources::table
                            .filter(schema::data_resources::session_id.eq(batch.session_id))
                            .filter(schema::data_resources::normalized_alias.eq(&normalized_alias))
                            .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                            .select(schema::data_resources::id).first::<String>(conn).optional()?.is_some();
                        if conflict {
                            let replacement_save_as_new = replacement
                                .as_ref()
                                .is_some_and(|(_, action)| action == "save_as_new");
                            if input.alias_explicit
                                || (candidate.inferred_schema_ref.is_some()
                                    && !replacement_save_as_new)
                            {
                                return Err(RepoError::InvalidState("resource_alias_conflict".into()));
                            } else {
                                alias = format!("data_{}", &input.resource_id[input.resource_id.len().saturating_sub(8)..]);
                                normalized_alias = alias.clone();
                            }
                        }
                    }
                    let record = DataResourceRecord {
                        id: input.resource_id,
                        session_id: batch.session_id,
                        display_name: input.display_name,
                        alias,
                        normalized_alias,
                        status: "live".into(),
                        declared_schema_ref: candidate.inferred_schema_ref.clone(),
                        source_kind: input.source_kind,
                        normalized_artifact_id: Some(artifact_id),
                        source_artifact_id,
                        schema_json,
                        schema_hash,
                        row_count,
                        source_batch_id: Some(batch_id),
                        source_model_version_id: None,
                        replaced_by_resource_id: None,
                        lineage_json,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                        deleted_at_ms: None,
                        tombstone_json: None,
                    };
                    diesel::insert_into(schema::data_resources::table).values(&record).execute(conn)?;
                    if let Some(old_id) = replacement.as_ref().filter(|(_, action)| action == "replace").map(|(id, _)| id) {
                        diesel::update(schema::data_resources::table.find(old_id))
                            .set(schema::data_resources::replaced_by_resource_id.eq(Some(record.id.clone())))
                            .execute(conn)?;
                    }
                    diesel::update(schema::upload_candidates::table.find(&candidate.id))
                        .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None)).execute(conn)?;
                    resources.push(record);
                }
                for (artifact_id, (resource_count, file_ids)) in source_use_counts {
                    let additional_refs = resource_count - file_ids.len() as i64;
                    if additional_refs > 0 {
                        diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+?1 WHERE storage_scope_id=(SELECT storage_scope_id FROM users WHERE id=?2) AND artifact_id=?3")
                            .bind::<BigInt,_>(additional_refs)
                            .bind::<Text,_>(&user)
                            .bind::<Integer,_>(artifact_id)
                            .execute(conn)?;
                    }
                    for file_id in file_ids {
                        diesel::update(schema::upload_files::table.find(file_id))
                            .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                            .execute(conn)?;
                    }
                }
                let receipt = serde_json::json!({"resourceIds":resources.iter().map(|r|&r.id).collect::<Vec<_>>()}).to_string();
                diesel::update(schema::upload_batches::table.find(batch_id)).set((
                    schema::upload_batches::resource_publish_status.eq("published"),
                    schema::upload_batches::resource_receipt_json.eq(Some(receipt)),
                    schema::upload_batches::resource_diagnostics_json.eq("[]"),
                    schema::upload_batches::updated_at_ms.eq(now_ms),
                )).execute(conn)?;
                Ok(PublishChildResult { resources, replaced_artifact_ids })
            })
        }).await.map_err(RepoError::Join)?
    }

    pub async fn publish_models(
        &self,
        user: &UserId,
        batch_id: i32,
        expected_draft: Option<DraftRef>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                let batch = owned_batch(conn, &user, batch_id)?;
                if batch.model_publish_status == "published" { return Ok(()); }
                let candidates = schema::upload_candidates::table
                    .filter(schema::upload_candidates::batch_id.eq(batch_id))
                    .filter(schema::upload_candidates::selected.eq(true))
                    .filter(schema::upload_candidates::purpose.eq("model_table"))
                    .load::<UploadCandidateRecord>(conn)?;
                if candidates.is_empty() { return Ok(()); }
                let scope_id = schema::users::table
                    .find(&user)
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                diesel::sql_query("INSERT INTO workspaces(session_id,created_at_ms,updated_at_ms) VALUES(?1,?2,?2) ON CONFLICT(session_id) DO NOTHING")
                    .bind::<Integer,_>(batch.session_id).bind::<BigInt,_>(now_ms).execute(conn)?;
                #[derive(QueryableByName)] struct Workspace { #[diesel(sql_type=Integer)] id:i32, #[diesel(sql_type=diesel::sql_types::Nullable<Integer>)] active_version_id:Option<i32> }
                let workspace = diesel::sql_query("SELECT id,active_version_id FROM workspaces WHERE session_id=?1")
                    .bind::<Integer,_>(batch.session_id).get_result::<Workspace>(conn)?;
                #[derive(QueryableByName)] struct Draft { #[diesel(sql_type=Integer)] id:i32, #[diesel(sql_type=BigInt)] revision:i64 }
                let draft = diesel::sql_query("SELECT id,revision FROM workspace_drafts WHERE workspace_id=?1")
                    .bind::<Integer,_>(workspace.id).get_result::<Draft>(conn).optional()?;
                if let Some(expected) = expected_draft {
                    let actual = draft.as_ref().map(|draft| DraftRef {
                        id: draft.id,
                        revision: draft.revision,
                    });
                    if actual != Some(expected) {
                        return Err(RepoError::InvalidState("draft_revision_conflict".into()));
                    }
                }
                let (draft_id, revision) = if let Some(draft) = draft { (draft.id, draft.revision + 1) } else {
                    diesel::sql_query("INSERT INTO workspace_drafts(workspace_id,base_version_id,revision,diagnostics_json,created_at_ms,updated_at_ms) VALUES(?1,?2,1,'{}',?3,?3)")
                        .bind::<Integer,_>(workspace.id).bind::<diesel::sql_types::Nullable<Integer>,_>(workspace.active_version_id).bind::<BigInt,_>(now_ms).execute(conn)?;
                    let id = diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()")).get_result(conn)?;
                    if let Some(version_id) = workspace.active_version_id {
                        diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) SELECT ?1,table_name,artifact_id,schema_json,row_count,content_hash,?2 FROM model_dataframes WHERE version_id=?3")
                            .bind::<Integer,_>(id).bind::<BigInt,_>(now_ms).bind::<Integer,_>(version_id).execute(conn)?;
                        diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+(SELECT count(*) FROM model_dataframes WHERE version_id=?2 AND artifact_id=storage_scope_artifacts.artifact_id) WHERE storage_scope_id=(SELECT storage_scope_id FROM users WHERE id=?1) AND artifact_id IN(SELECT artifact_id FROM model_dataframes WHERE version_id=?2)")
                            .bind::<Text,_>(&user).bind::<Integer,_>(version_id).execute(conn)?;
                    }
                    (id, 1)
                };
                let mut release = Vec::new();
                for candidate in &candidates {
                    let table_name = candidate.normalized_alias.clone().or_else(|| candidate.proposed_alias.clone()).ok_or_else(|| RepoError::InvalidState("模型候选缺少表名".into()))?;
                    if table_name.eq_ignore_ascii_case("para") { return Err(RepoError::InvalidState("para 不能进入模型草稿".into())); }
                    let artifact_id = candidate.normalized_artifact_id.ok_or_else(|| RepoError::InvalidState("模型候选缺少 artifact".into()))?;
                    let content_hash = schema::artifacts::table
                        .find(artifact_id)
                        .select(schema::artifacts::sha256)
                        .first::<String>(conn)?;
                    let old = schema::draft_dataframes::table.filter(schema::draft_dataframes::draft_id.eq(draft_id)).filter(schema::draft_dataframes::table_name.eq(&table_name))
                        .select(schema::draft_dataframes::artifact_id).first::<i32>(conn).optional()?;
                    diesel::delete(schema::draft_dataframes::table.filter(schema::draft_dataframes::draft_id.eq(draft_id)).filter(schema::draft_dataframes::table_name.eq(&table_name))).execute(conn)?;
                    diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                        .bind::<Integer,_>(draft_id).bind::<Text,_>(&table_name).bind::<Integer,_>(artifact_id)
                        .bind::<Text,_>(candidate.schema_json.clone().ok_or_else(|| RepoError::InvalidState("模型候选缺少 schema".into()))?)
                        .bind::<BigInt,_>(candidate.row_count.ok_or_else(|| RepoError::InvalidState("模型候选缺少行数".into()))?)
                        .bind::<Text,_>(content_hash)
                        .bind::<BigInt,_>(now_ms).execute(conn)?;
                    if let Some(old) = old { release.push(old); }
                    diesel::update(schema::upload_candidates::table.find(&candidate.id)).set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None)).execute(conn)?;
                }
                diesel::update(schema::workspace_drafts::table.find(draft_id)).set((schema::workspace_drafts::revision.eq(revision),schema::workspace_drafts::updated_at_ms.eq(now_ms))).execute(conn)?;
                super::artifact::release_refs(conn, scope_id, release, now_ms)?;
                let receipt = serde_json::json!({"draftId":draft_id,"draftRevision":revision,"tableCount":candidates.len()}).to_string();
                diesel::update(schema::upload_batches::table.find(batch_id)).set((schema::upload_batches::model_publish_status.eq("published"),schema::upload_batches::model_receipt_json.eq(Some(receipt)),schema::upload_batches::model_diagnostics_json.eq("[]"),schema::upload_batches::updated_at_ms.eq(now_ms))).execute(conn)?;
                Ok(())
            })
        }).await.map_err(RepoError::Join)?
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
            let batch = owned_batch(&mut conn, &user, batch_id)?;
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
        tokio::task::spawn_blocking(move || { let mut conn=pool.get().map_err(RepoError::Pool)?; conn.immediate_transaction(|conn| {
            let batch=owned_batch(conn,&user,batch_id)?;
            if matches!(batch.status.as_str(), "discarded" | "expired" | "confirmed" | "failed") {
                return Ok(FinishedUploadBatch { staged_paths: Vec::new() });
            }
            let model_done=matches!(batch.model_publish_status.as_str(),"published"|"not_applicable");
            let resource_done=matches!(batch.resource_publish_status.as_str(),"published"|"not_applicable");
            let any_done = batch.model_publish_status == "published"
                || batch.resource_publish_status == "published";
            let status = if model_done && resource_done {
                "confirmed"
            } else if any_done {
                "partially_confirmed"
            } else {
                "ready_to_confirm"
            };
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
                .collect::<std::collections::BTreeSet<_>>();
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
                .filter(schema::upload_candidates::id.ne_all(retained_candidate_ids.iter().cloned()))
                .filter(schema::upload_candidates::normalized_artifact_id.is_not_null())
                .select(schema::upload_candidates::normalized_artifact_id)
                .load::<Option<i32>>(conn)?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let releasable_source_ids = files
                .iter()
                .filter(|file| !retained_file_ids.contains(&file.id))
                .filter_map(|file| file.source_artifact_id)
                .collect::<Vec<_>>();
            release_artifact_ids.extend(releasable_source_ids);
            let scope_id = schema::sessions::table
                .inner_join(schema::users::table.on(schema::users::id.eq(schema::sessions::user_id)))
                .filter(schema::sessions::id.eq(batch.session_id))
                .filter(schema::sessions::user_id.eq(&user))
                .select(schema::users::storage_scope_id)
                .first::<i32>(conn)?;
            super::artifact::release_refs(conn, scope_id, release_artifact_ids, now_ms)?;
            if !retained_candidate_ids.is_empty() {
                diesel::update(
                    schema::upload_candidates::table
                        .filter(schema::upload_candidates::batch_id.eq(batch_id))
                        .filter(schema::upload_candidates::id.ne_all(retained_candidate_ids.iter().cloned())),
                )
                .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None))
                .execute(conn)?;
            } else {
                diesel::update(schema::upload_candidates::table.filter(schema::upload_candidates::batch_id.eq(batch_id)))
                    .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None))
                    .execute(conn)?;
            }
            if !retained_file_ids.is_empty() {
                diesel::update(
                    schema::upload_files::table
                        .filter(schema::upload_files::batch_id.eq(batch_id))
                        .filter(schema::upload_files::id.ne_all(retained_file_ids.iter().cloned())),
                )
                .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                .execute(conn)?;
            } else {
                diesel::update(schema::upload_files::table.filter(schema::upload_files::batch_id.eq(batch_id)))
                    .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                    .execute(conn)?;
            }
            if released_bytes > 0 {
                diesel::sql_query("UPDATE storage_scopes SET reserved_bytes=reserved_bytes-?1,updated_at_ms=?2 WHERE id=(SELECT u.storage_scope_id FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.id=?3)")
                    .bind::<BigInt,_>(released_bytes).bind::<BigInt,_>(now_ms).bind::<Integer,_>(batch.session_id).execute(conn)?;
            }
            diesel::update(schema::upload_batches::table.find(batch_id)).set((schema::upload_batches::status.eq(status),schema::upload_batches::reserved_bytes.eq(retained_bytes),schema::upload_batches::updated_at_ms.eq(now_ms))).execute(conn)?;
            Ok(FinishedUploadBatch { staged_paths: paths })
        }) }).await.map_err(RepoError::Join)?
    }

    pub async fn mark_needs_rename(&self, user: &UserId, id: &str, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_resource(&mut conn, &user, &id, false)?;
            diesel::update(schema::data_resources::table.find(&id))
                .filter(schema::data_resources::status.eq("live"))
                .set((
                    schema::data_resources::status.eq("needs_rename"),
                    schema::data_resources::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        user: &UserId,
        id: &str,
        display_name: Option<String>,
        alias: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let current = owned_resource(&mut conn, &user, &id, false)?;
            if let Some((_, normalized)) = &alias {
                let conflict = schema::data_resources::table
                    .filter(schema::data_resources::session_id.eq(current.session_id))
                    .filter(schema::data_resources::normalized_alias.eq(normalized))
                    .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                    .filter(schema::data_resources::id.ne(&id))
                    .select(schema::data_resources::id)
                    .first::<String>(&mut conn)
                    .optional()?
                    .is_some();
                if conflict {
                    return Err(RepoError::InvalidState("resource_alias_conflict".into()));
                }
            }
            diesel::update(schema::data_resources::table.find(&id))
                .set((
                    schema::data_resources::display_name
                        .eq(display_name.unwrap_or(current.display_name)),
                    schema::data_resources::alias
                        .eq(alias.as_ref().map(|v| v.0.clone()).unwrap_or(current.alias)),
                    schema::data_resources::normalized_alias
                        .eq(alias.map(|v| v.1).unwrap_or(current.normalized_alias)),
                    schema::data_resources::status.eq("live"),
                    schema::data_resources::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            schema::data_resources::table
                .find(id)
                .first(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn promote_dataset(
        &self,
        user: &UserId,
        input: PromoteDatasetInput,
    ) -> RepoResult<DataResourceRecord> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let PromoteDatasetInput {
            session_id,
            dataset_id,
            resource_id,
            display_name,
            alias,
            normalized_alias,
            now_ms,
        } = input;
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                ensure_session(conn, &user, session_id)?;
                #[derive(QueryableByName)]
                struct Dataset {
                    #[diesel(sql_type = Integer)]
                    artifact_id: i32,
                    #[diesel(sql_type = Text)]
                    result_name: String,
                    #[diesel(sql_type = Text)]
                    schema_json: String,
                    #[diesel(sql_type = Text)]
                    schema_hash: String,
                    #[diesel(sql_type = BigInt)]
                    row_count: i64,
                    #[diesel(sql_type = diesel::sql_types::Nullable<Integer>)]
                    run_id: Option<i32>,
                    #[diesel(sql_type = diesel::sql_types::Nullable<Integer>)]
                    analysis_run_id: Option<i32>,
                    #[diesel(sql_type = Integer)]
                    source_session_id: i32,
                }
                let dataset = diesel::sql_query("SELECT d.artifact_id,d.result_name,d.schema_json,d.schema_hash,d.row_count,d.run_id,d.analysis_run_id,COALESCE(r.session_id,a.session_id) source_session_id FROM result_datasets d LEFT JOIN simulation_runs r ON r.id=d.run_id LEFT JOIN result_analysis_runs a ON a.id=d.analysis_run_id JOIN sessions s ON s.id=COALESCE(r.session_id,a.session_id) WHERE d.id=?1 AND s.user_id=?2 AND d.deleted_at_ms IS NULL AND d.artifact_id IS NOT NULL")
                    .bind::<Integer, _>(dataset_id)
                    .bind::<Text, _>(&user)
                    .get_result::<Dataset>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if dataset.source_session_id != session_id {
                    return Err(RepoError::NotFound);
                }
                let mut final_alias = alias;
                let mut final_normalized = normalized_alias;
                let conflict = schema::data_resources::table
                    .filter(schema::data_resources::session_id.eq(session_id))
                    .filter(schema::data_resources::normalized_alias.eq(&final_normalized))
                    .filter(schema::data_resources::status.ne("deleted"))
                    .select(schema::data_resources::id)
                    .first::<String>(conn)
                    .optional()?
                    .is_some();
                if conflict {
                    final_alias = format!(
                        "data_{}",
                        &resource_id[resource_id.len().saturating_sub(8)..]
                    );
                    final_normalized = final_alias.clone();
                }
                let source_kind = if dataset.analysis_run_id.is_some() {
                    "analysis"
                } else {
                    "result"
                };
                let record = DataResourceRecord {
                    id: resource_id,
                    session_id,
                    display_name,
                    alias: final_alias,
                    normalized_alias: final_normalized,
                    status: "live".into(),
                    declared_schema_ref: None,
                    source_kind: source_kind.into(),
                    normalized_artifact_id: Some(dataset.artifact_id),
                    source_artifact_id: None,
                    schema_json: dataset.schema_json,
                    schema_hash: dataset.schema_hash,
                    row_count: dataset.row_count,
                    source_batch_id: None,
                    source_model_version_id: None,
                    replaced_by_resource_id: None,
                    lineage_json: serde_json::json!({
                        "datasetId": dataset_id,
                        "runId": dataset.run_id,
                        "analysisRunId": dataset.analysis_run_id,
                        "resultName": dataset.result_name,
                    })
                    .to_string(),
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                    deleted_at_ms: None,
                    tombstone_json: None,
                };
                diesel::insert_into(schema::data_resources::table)
                    .values(&record)
                    .execute(conn)?;
                diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+1 WHERE storage_scope_id=(SELECT storage_scope_id FROM users WHERE id=?1) AND artifact_id=?2")
                    .bind::<Text, _>(&user)
                    .bind::<Integer, _>(dataset.artifact_id)
                    .execute(conn)?;
                Ok(record)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn store_structured_preview(&self, input: StructuredPreviewInput) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                owned_resource(conn, &input.user_id, &input.old_resource_id, false)?;
                let superseded = schema::data_resource_replacement_previews::table
                    .filter(schema::data_resource_replacement_previews::user_id.eq(&input.user_id))
                    .filter(
                        schema::data_resource_replacement_previews::expires_at_ms
                            .lt(input.created_at_ms)
                            .or(schema::data_resource_replacement_previews::old_resource_id
                                .eq(&input.old_resource_id)),
                    )
                    .select(schema::data_resource_replacement_previews::pending_artifact_id)
                    .load::<i32>(conn)?;
                let scope_id = schema::users::table
                    .filter(schema::users::id.eq(&input.user_id))
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                super::artifact::release_refs(
                    conn,
                    scope_id,
                    superseded,
                    input.created_at_ms,
                )?;
                diesel::sql_query("DELETE FROM data_resource_replacement_previews WHERE user_id=?2 AND (expires_at_ms<?1 OR old_resource_id=?3)")
                    .bind::<BigInt, _>(input.created_at_ms)
                    .bind::<Text, _>(&input.user_id)
                    .bind::<Text, _>(&input.old_resource_id)
                    .execute(conn)?;
                diesel::insert_into(schema::data_resource_replacement_previews::table)
                    .values(DataResourceReplacementPreviewRecord {
                        token_hash: input.token_hash,
                        user_id: input.user_id,
                        old_resource_id: input.old_resource_id,
                        pending_artifact_id: input.pending_artifact_id,
                        pending_resource_id: input.pending_resource_id,
                        expected_content_hash: input.expected_content_hash,
                        schema_ref: input.schema_ref,
                        schema_json: input.schema_json,
                        schema_hash: input.schema_hash,
                        values_json: input.values_json,
                        display_name: input.display_name,
                        proposed_alias: input.proposed_alias,
                        impact_hash: input.impact_hash,
                        impact_json: input.impact_json,
                        expires_at_ms: input.expires_at_ms,
                        created_at_ms: input.created_at_ms,
                    })
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn confirm_structured_preview(
        &self,
        user: &UserId,
        token_hash: String,
        action: String,
        now: i64,
    ) -> RepoResult<StructuredPreviewConfirmation> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let scope_id = schema::users::table
                    .filter(schema::users::id.eq(&user))
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                let preview = schema::data_resource_replacement_previews::table
                    .find(&token_hash)
                    .filter(schema::data_resource_replacement_previews::user_id.eq(&user))
                    .first::<DataResourceReplacementPreviewRecord>(conn)
                    .optional()?
                    .ok_or_else(|| RepoError::InvalidState("replacement_preview_invalid".into()))?;
                if preview.expires_at_ms < now {
                    super::artifact::release_refs(
                        conn,
                        scope_id,
                        [preview.pending_artifact_id],
                        now,
                    )?;
                    diesel::delete(
                        schema::data_resource_replacement_previews::table.find(&token_hash),
                    )
                    .execute(conn)?;
                    return Ok(StructuredPreviewConfirmation::Rejected {
                        code: "replacement_preview_invalid",
                    });
                }
                let old = owned_resource(conn, &user, &preview.old_resource_id, true)?;
                let Some(old_artifact_id) = old.normalized_artifact_id else {
                    super::artifact::release_refs(
                        conn,
                        scope_id,
                        [preview.pending_artifact_id],
                        now,
                    )?;
                    diesel::delete(
                        schema::data_resource_replacement_previews::table.find(&token_hash),
                    )
                    .execute(conn)?;
                    return Ok(StructuredPreviewConfirmation::Rejected {
                        code: "resource_stale",
                    });
                };
                let current_hash = schema::artifacts::table
                    .find(old_artifact_id)
                    .select(schema::artifacts::sha256)
                    .first::<String>(conn)?;
                if !matches!(old.status.as_str(), "live" | "needs_rename")
                    || current_hash != preview.expected_content_hash
                {
                    super::artifact::release_refs(
                        conn,
                        scope_id,
                        [preview.pending_artifact_id],
                        now,
                    )?;
                    diesel::delete(
                        schema::data_resource_replacement_previews::table.find(&token_hash),
                    )
                    .execute(conn)?;
                    return Ok(StructuredPreviewConfirmation::Rejected {
                        code: "resource_stale",
                    });
                }
                if action == "cancel" {
                    super::artifact::release_refs(
                        conn,
                        scope_id,
                        [preview.pending_artifact_id],
                        now,
                    )?;
                    diesel::delete(
                        schema::data_resource_replacement_previews::table.find(&token_hash),
                    )
                    .execute(conn)?;
                    return Ok(StructuredPreviewConfirmation::Cancelled);
                }

                let mut alias = preview.proposed_alias.clone();
                let mut normalized = alias.to_ascii_lowercase();
                if action == "replace" {
                    alias = old.alias.clone();
                    normalized = old.normalized_alias.clone();
                    let backup = format!("data_{}", &old.id[old.id.len().saturating_sub(8)..]);
                    diesel::update(schema::data_resources::table.find(&old.id))
                        .set((
                            schema::data_resources::alias.eq(&backup),
                            schema::data_resources::normalized_alias.eq(&backup),
                            schema::data_resources::status.eq("replaced"),
                            schema::data_resources::updated_at_ms.eq(now),
                        ))
                        .execute(conn)?;
                    diesel::sql_query("UPDATE study_configurations SET status='invalidated',updated_at_ms=?1 WHERE id IN(SELECT configuration_id FROM study_input_bindings WHERE resource_id=?2) AND status IN ('choosing_facade','collecting_inputs','ready_to_confirm')")
                        .bind::<BigInt,_>(now)
                        .bind::<Text,_>(&old.id)
                        .execute(conn)?;
                } else {
                    let conflict = schema::data_resources::table
                        .filter(schema::data_resources::session_id.eq(old.session_id))
                        .filter(schema::data_resources::normalized_alias.eq(&normalized))
                        .filter(schema::data_resources::status.ne("deleted"))
                        .select(schema::data_resources::id)
                        .first::<String>(conn)
                        .optional()?
                        .is_some();
                    if conflict {
                        alias = format!(
                            "data_{}",
                            &preview.pending_resource_id
                                [preview.pending_resource_id.len().saturating_sub(8)..]
                        );
                        normalized = alias.clone();
                    }
                }
                let record = DataResourceRecord {
                    id: preview.pending_resource_id,
                    session_id: old.session_id,
                    display_name: preview.display_name,
                    alias,
                    normalized_alias: normalized,
                    status: "live".into(),
                    declared_schema_ref: Some(preview.schema_ref),
                    source_kind: "struct_edit".into(),
                    normalized_artifact_id: Some(preview.pending_artifact_id),
                    source_artifact_id: None,
                    schema_json: preview.schema_json,
                    schema_hash: preview.schema_hash,
                    row_count: 1,
                    source_batch_id: None,
                    source_model_version_id: old.source_model_version_id,
                    replaced_by_resource_id: None,
                    lineage_json: serde_json::json!({
                        "replacesResourceId": old.id,
                        "replacementAction": action,
                        "values": serde_json::from_str::<serde_json::Value>(&preview.values_json)
                            .unwrap_or_default(),
                        "diff": serde_json::from_str::<serde_json::Value>(&preview.impact_json)
                            .unwrap_or_default(),
                    })
                    .to_string(),
                    created_at_ms: now,
                    updated_at_ms: now,
                    deleted_at_ms: None,
                    tombstone_json: None,
                };
                diesel::insert_into(schema::data_resources::table)
                    .values(&record)
                    .execute(conn)?;
                if action == "replace" {
                    diesel::update(schema::data_resources::table.find(&old.id))
                        .set(
                            schema::data_resources::replaced_by_resource_id
                                .eq(Some(record.id.clone())),
                        )
                        .execute(conn)?;
                }
                diesel::delete(schema::data_resource_replacement_previews::table.find(&token_hash))
                    .execute(conn)?;
                Ok(StructuredPreviewConfirmation::Published(Box::new(record)))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn take_expired_structured_previews(&self, now_ms: i64) -> RepoResult<usize> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                #[derive(QueryableByName)]
                struct Row {
                    #[diesel(sql_type = Integer)]
                    artifact_id: i32,
                    #[diesel(sql_type = Integer)]
                    storage_scope_id: i32,
                }
                let expired = diesel::sql_query(
                    "SELECT p.pending_artifact_id artifact_id,u.storage_scope_id
                     FROM data_resource_replacement_previews p
                     JOIN users u ON u.id=p.user_id
                     WHERE p.expires_at_ms<?1",
                )
                .bind::<BigInt, _>(now_ms)
                .load::<Row>(conn)?;
                let count = expired.len();
                let mut by_scope = BTreeMap::<i32, Vec<i32>>::new();
                for row in expired {
                    by_scope
                        .entry(row.storage_scope_id)
                        .or_default()
                        .push(row.artifact_id);
                }
                for (scope_id, artifact_ids) in by_scope {
                    super::artifact::release_refs(conn, scope_id, artifact_ids, now_ms)?;
                }
                diesel::sql_query(
                    "DELETE FROM data_resource_replacement_previews WHERE expires_at_ms<?1",
                )
                .bind::<BigInt, _>(now_ms)
                .execute(conn)?;
                Ok(count)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn deletion_impact(&self, user: &UserId, id: &str) -> RepoResult<DeletionImpactRow> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let resource = owned_resource(&mut conn, &user, &id, false)?;
            let artifact_id = resource
                .normalized_artifact_id
                .ok_or_else(|| RepoError::InvalidState("资源 artifact 已释放".into()))?;
            let size = schema::artifacts::table
                .find(artifact_id)
                .select(schema::artifacts::size_bytes)
                .first(&mut conn)?;
            let pending = diesel::sql_query("SELECT count(*) value FROM study_input_bindings b JOIN study_configurations c ON c.id=b.configuration_id WHERE b.resource_id=?1 AND c.status IN ('choosing_facade','collecting_inputs','ready_to_confirm')")
                .bind::<Text, _>(&id)
                .get_result::<Count>(&mut conn)?
                .value;
            let frozen = diesel::sql_query(
                "SELECT count(*) value FROM workflow_input_snapshots WHERE resource_id=?1",
            )
            .bind::<Text, _>(&id)
            .get_result::<Count>(&mut conn)?
            .value;
            let historical = diesel::sql_query("SELECT (SELECT count(*) FROM workflow_input_snapshots w JOIN managed_workflows m ON m.id=w.workflow_id WHERE w.resource_id=?1 AND m.status IN ('succeeded','failed','cancelled_by_user','service_restarted')) + (SELECT count(*) FROM result_analysis_inputs WHERE resource_id=?1) + (SELECT count(*) FROM dataset_lineage_snapshots WHERE source_resource_id=?1) value")
                .bind::<Text, _>(&id)
                .get_result::<Count>(&mut conn)?
                .value;
            Ok(DeletionImpactRow {
                resource,
                normalized_size_bytes: size,
                pending_configuration_count: pending,
                frozen_reference_count: frozen,
                historical_reference_count: historical,
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn store_deletion_preview(
        &self,
        user: &UserId,
        input: DeletionPreviewInput,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let DeletionPreviewInput {
            resource_id,
            token_hash,
            impact_hash,
            impact_json,
            expires_at_ms,
            created_at_ms,
        } = input;
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            owned_resource(&mut conn, &user, &resource_id, false)?;
            diesel::sql_query("DELETE FROM data_resource_deletion_previews WHERE expires_at_ms<?1 OR (user_id=?2 AND resource_id=?3)")
                .bind::<BigInt, _>(created_at_ms)
                .bind::<Text, _>(&user)
                .bind::<Text, _>(&resource_id)
                .execute(&mut conn)?;
            diesel::sql_query("INSERT INTO data_resource_deletion_previews(token_hash,user_id,resource_id,impact_hash,impact_json,expires_at_ms,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                .bind::<Text, _>(token_hash)
                .bind::<Text, _>(user)
                .bind::<Text, _>(resource_id)
                .bind::<Text, _>(impact_hash)
                .bind::<Text, _>(impact_json)
                .bind::<BigInt, _>(expires_at_ms)
                .bind::<BigInt, _>(created_at_ms)
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(
        &self,
        user: &UserId,
        id: &str,
        token_hash: String,
        current_impact_hash: String,
        now: i64,
    ) -> RepoResult<DeletedResource> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let resource = owned_resource(conn, &user, &id, true)?;
                if resource.status == "deleted" {
                    return Ok(DeletedResource { resource });
                }
                #[derive(QueryableByName)]
                struct Preview {
                    #[diesel(sql_type = Text)]
                    impact_hash: String,
                }
                let preview = diesel::sql_query("SELECT impact_hash FROM data_resource_deletion_previews WHERE token_hash=?1 AND user_id=?2 AND resource_id=?3 AND expires_at_ms>=?4")
                    .bind::<Text,_>(&token_hash)
                    .bind::<Text,_>(&user)
                    .bind::<Text,_>(&id)
                    .bind::<BigInt,_>(now)
                    .get_result::<Preview>(conn)
                    .optional()?
                    .ok_or_else(|| RepoError::InvalidState("invalid_deletion_impact_token".into()))?;
                if preview.impact_hash != current_impact_hash {
                    return Err(RepoError::InvalidState("deletion_impact_changed".into()));
                }
                diesel::sql_query("UPDATE study_configurations SET status='invalidated',updated_at_ms=?1 WHERE id IN(SELECT configuration_id FROM study_input_bindings WHERE resource_id=?2) AND status IN ('choosing_facade','collecting_inputs','ready_to_confirm')")
                    .bind::<BigInt,_>(now)
                    .bind::<Text,_>(&id)
                    .execute(conn)?;
                let tombstone = serde_json::json!({
                    "id": id,
                    "alias": resource.alias,
                    "schemaHash": resource.schema_hash,
                    "deletedAtMs": now,
                })
                .to_string();
                let mut release_artifact_ids = resource
                    .normalized_artifact_id
                    .into_iter()
                    .collect::<Vec<_>>();
                if let Some(source_artifact_id) = resource.source_artifact_id {
                    release_artifact_ids.push(source_artifact_id);
                    let other_sources = schema::data_resources::table
                        .filter(schema::data_resources::source_artifact_id.eq(source_artifact_id))
                        .filter(schema::data_resources::id.ne(&id))
                        .filter(schema::data_resources::status.ne("deleted"))
                        .select(schema::data_resources::id)
                        .first::<String>(conn)
                        .optional()?
                        .is_some();
                    if !other_sources {
                        diesel::update(
                            schema::upload_files::table
                                .filter(schema::upload_files::source_artifact_id.eq(source_artifact_id)),
                        )
                        .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                        .execute(conn)?;
                    }
                }
                let scope_id = schema::users::table
                    .filter(schema::users::id.eq(&user))
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                super::artifact::release_refs(
                    conn,
                    scope_id,
                    release_artifact_ids,
                    now,
                )?;
                diesel::update(schema::data_resources::table.find(&id))
                    .set((
                        schema::data_resources::status.eq("deleted"),
                        schema::data_resources::normalized_artifact_id.eq(Option::<i32>::None),
                        schema::data_resources::source_artifact_id.eq(Option::<i32>::None),
                        schema::data_resources::deleted_at_ms.eq(Some(now)),
                        schema::data_resources::tombstone_json.eq(Some(tombstone)),
                        schema::data_resources::updated_at_ms.eq(now),
                    ))
                    .execute(conn)?;
                diesel::delete(
                    schema::data_resource_deletion_previews::table
                        .filter(schema::data_resource_deletion_previews::resource_id.eq(&id)),
                )
                .execute(conn)?;
                Ok(DeletedResource {
                    resource,
                })
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    value: i64,
}

fn ensure_session(conn: &mut SqliteConnection, user: &str, session_id: i32) -> RepoResult<()> {
    let exists = schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(RepoError::NotFound)
    }
}
fn owned_batch(conn: &mut SqliteConnection, user: &str, id: i32) -> RepoResult<UploadBatchRecord> {
    schema::upload_batches::table
        .inner_join(
            schema::sessions::table.on(schema::sessions::id.eq(schema::upload_batches::session_id)),
        )
        .filter(schema::upload_batches::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::upload_batches::all_columns)
        .first(conn)
        .optional()?
        .ok_or(RepoError::NotFound)
}
fn owned_resource(
    conn: &mut SqliteConnection,
    user: &str,
    id: &str,
    historical: bool,
) -> RepoResult<DataResourceRecord> {
    let mut query = schema::data_resources::table
        .inner_join(
            schema::sessions::table.on(schema::sessions::id.eq(schema::data_resources::session_id)),
        )
        .filter(schema::data_resources::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::data_resources::all_columns)
        .into_boxed();
    if !historical {
        query = query.filter(schema::data_resources::status.eq_any(["live", "needs_rename"]));
    }
    query.first(conn).optional()?.ok_or(RepoError::NotFound)
}
fn enrich_resource(
    conn: &mut SqliteConnection,
    resource: DataResourceRecord,
) -> RepoResult<ResourcePageRow> {
    let normalized_artifact_id = resource
        .normalized_artifact_id
        .ok_or_else(|| RepoError::InvalidState("资源 artifact 已释放".into()))?;
    let normalized = schema::artifacts::table
        .find(normalized_artifact_id)
        .select((schema::artifacts::sha256, schema::artifacts::size_bytes))
        .first::<(String, i64)>(conn)?;
    let source = resource
        .source_artifact_id
        .map(|id| {
            schema::artifacts::table
                .find(id)
                .select((schema::artifacts::sha256, schema::artifacts::size_bytes))
                .first::<(String, i64)>(conn)
        })
        .transpose()?;
    Ok(ResourcePageRow {
        resource,
        normalized_sha256: normalized.0,
        normalized_size_bytes: normalized.1,
        source_sha256: source.as_ref().map(|v| v.0.clone()),
        source_size_bytes: source.map(|v| v.1),
    })
}
