use std::collections::{BTreeMap, BTreeSet};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use crate::auth::UserId;
use crate::db::models::{DataResourceRecord, DraftRef, UploadCandidateRecord, UploadFileRecord};
use crate::db::schema;

use super::super::data_resource::owned_batch;
use super::super::{RepoError, RepoResult};
use super::{PublishChildResult, ResourcePublishInput};

pub(in crate::db::repositories) fn publish_resources_on(
    conn: &mut diesel::SqliteConnection,
    user: &str,
    batch_id: i32,
    inputs: Vec<ResourcePublishInput>,
    replacement: Option<(String, String)>,
    now_ms: i64,
) -> RepoResult<PublishChildResult> {
    let batch = owned_batch(conn, user, batch_id)?;
    if batch.resource_publish_status == "published" {
        let rows = schema::data_resources::table
            .filter(schema::data_resources::source_batch_id.eq(batch_id))
            .load(conn)?;
        return Ok(PublishChildResult {
            resources: rows,
            replaced_artifact_ids: vec![],
        });
    }
    if !matches!(
        batch.status.as_str(),
        "ready_to_confirm" | "partially_confirmed"
    ) {
        return Err(RepoError::InvalidState("上传批次尚未准备确认".into()));
    }
    let candidates = schema::upload_candidates::table
        .filter(schema::upload_candidates::batch_id.eq(batch_id))
        .filter(schema::upload_candidates::selected.eq(true))
        .filter(schema::upload_candidates::purpose.eq("data_resource"))
        .load::<UploadCandidateRecord>(conn)?;
    if candidates.len() != inputs.len()
        || inputs
            .iter()
            .any(|input| !candidates.iter().any(|c| c.id == input.candidate_id))
    {
        return Err(RepoError::InvalidState("资源候选决定不完整".into()));
    }
    let files = schema::upload_files::table
        .filter(schema::upload_files::batch_id.eq(batch_id))
        .load::<UploadFileRecord>(conn)?;
    let mut replaced_artifact_ids = Vec::new();
    let mut old_replacement = None;
    let mut replacement_lineage = None;
    if let Some((old_id, action)) = &replacement {
        if matches!(action.as_str(), "replace" | "save_as_new") {
            let old = schema::data_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::data_resources::session_id)),
                )
                .filter(schema::data_resources::id.eq(old_id))
                .filter(schema::sessions::user_id.eq(user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
                .select(schema::data_resources::all_columns)
                .first::<DataResourceRecord>(conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
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
            return Ok(PublishChildResult {
                resources: vec![],
                replaced_artifact_ids: vec![],
            });
        }
    }
    if old_replacement.is_some() && inputs.len() != 1 {
        return Err(RepoError::InvalidState("替换批次必须只发布一个资源".into()));
    }
    let mut resources = Vec::new();
    let mut source_use_counts = BTreeMap::<i32, (i64, BTreeSet<String>)>::new();
    for input in inputs {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.id == input.candidate_id)
            .ok_or(RepoError::NotFound)?;
        let artifact_id = candidate
            .normalized_artifact_id
            .ok_or_else(|| RepoError::InvalidState("候选缺少规范化 artifact".into()))?;
        let schema_json = candidate
            .schema_json
            .clone()
            .ok_or_else(|| RepoError::InvalidState("候选缺少 schema".into()))?;
        let schema_hash = candidate
            .schema_hash
            .clone()
            .ok_or_else(|| RepoError::InvalidState("候选缺少 schema hash".into()))?;
        let row_count = candidate
            .row_count
            .ok_or_else(|| RepoError::InvalidState("候选缺少行数".into()))?;
        let source_artifact_id = files
            .iter()
            .find(|file| file.id == candidate.file_id)
            .and_then(|file| file.source_artifact_id);
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
            diesel::update(schema::data_resources::table.find(&old.id))
                .set((
                    schema::data_resources::alias.eq(&backup_alias),
                    schema::data_resources::normalized_alias.eq(backup_alias.to_ascii_lowercase()),
                    schema::data_resources::status.eq("replaced"),
                    schema::data_resources::replaced_by_resource_id.eq(Option::<String>::None),
                    schema::data_resources::updated_at_ms.eq(now_ms),
                ))
                .execute(conn)?;
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
                .select(schema::data_resources::id)
                .first::<String>(conn)
                .optional()?
                .is_some();
            if conflict {
                let replacement_save_as_new = replacement
                    .as_ref()
                    .is_some_and(|(_, action)| action == "save_as_new");
                if input.alias_explicit
                    || (candidate.inferred_schema_ref.is_some() && !replacement_save_as_new)
                {
                    return Err(RepoError::InvalidState("resource_alias_conflict".into()));
                } else {
                    alias = format!(
                        "data_{}",
                        &input.resource_id[input.resource_id.len().saturating_sub(8)..]
                    );
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
        diesel::insert_into(schema::data_resources::table)
            .values(&record)
            .execute(conn)?;
        if let Some(old_id) = replacement
            .as_ref()
            .filter(|(_, action)| action == "replace")
            .map(|(id, _)| id)
        {
            diesel::update(schema::data_resources::table.find(old_id))
                .set(schema::data_resources::replaced_by_resource_id.eq(Some(record.id.clone())))
                .execute(conn)?;
        }
        diesel::update(schema::upload_candidates::table.find(&candidate.id))
            .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None))
            .execute(conn)?;
        resources.push(record);
    }
    let source_scope_id =
        super::super::artifact_refs::storage_scope_id_for_user(conn, &UserId(user.to_owned()))?;
    for (artifact_id, (resource_count, file_ids)) in source_use_counts {
        let additional_refs = resource_count - file_ids.len() as i64;
        if additional_refs > 0 {
            super::super::artifact_refs::retain_ref_count(
                conn,
                source_scope_id,
                artifact_id,
                additional_refs,
            )?;
        }
        for file_id in file_ids {
            diesel::update(schema::upload_files::table.find(file_id))
                .set(schema::upload_files::source_artifact_id.eq(Option::<i32>::None))
                .execute(conn)?;
        }
    }
    let receipt =
        serde_json::json!({"resourceIds":resources.iter().map(|r|&r.id).collect::<Vec<_>>()})
            .to_string();
    diesel::update(schema::upload_batches::table.find(batch_id))
        .set((
            schema::upload_batches::resource_publish_status.eq("published"),
            schema::upload_batches::resource_receipt_json.eq(Some(receipt)),
            schema::upload_batches::resource_diagnostics_json.eq("[]"),
            schema::upload_batches::updated_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    Ok(PublishChildResult {
        resources,
        replaced_artifact_ids,
    })
}

pub(in crate::db::repositories) fn apply_model_candidates_to_draft_on(
    conn: &mut diesel::SqliteConnection,
    user: &str,
    batch_id: i32,
    expected_draft: Option<DraftRef>,
    mode: super::DraftApplicationMode,
    now_ms: i64,
) -> RepoResult<()> {
    let batch = owned_batch(conn, user, batch_id)?;
    if batch.source_kind != mode.expected_source_kind() {
        return Err(RepoError::InvalidState("upload_strategy_mismatch".into()));
    }
    if batch.model_publish_status == "published" {
        return Ok(());
    }
    let candidates = schema::upload_candidates::table
        .filter(schema::upload_candidates::batch_id.eq(batch_id))
        .filter(schema::upload_candidates::selected.eq(true))
        .filter(schema::upload_candidates::purpose.eq("model_table"))
        .load::<UploadCandidateRecord>(conn)?;
    if candidates.is_empty() {
        return Ok(());
    }
    let scope_id =
        super::super::artifact_refs::storage_scope_id_for_user(conn, &UserId(user.to_owned()))?;
    diesel::sql_query("INSERT INTO workspaces(session_id,created_at_ms,updated_at_ms) VALUES(?1,?2,?2) ON CONFLICT(session_id) DO NOTHING")
                    .bind::<Integer,_>(batch.session_id).bind::<BigInt,_>(now_ms).execute(conn)?;
    #[derive(QueryableByName)]
    struct Workspace {
        #[diesel(sql_type=Integer)]
        id: i32,
        #[diesel(sql_type=diesel::sql_types::Nullable<Integer>)]
        active_version_id: Option<i32>,
    }
    let workspace =
        diesel::sql_query("SELECT id,active_version_id FROM workspaces WHERE session_id=?1")
            .bind::<Integer, _>(batch.session_id)
            .get_result::<Workspace>(conn)?;
    #[derive(QueryableByName)]
    struct Draft {
        #[diesel(sql_type=Integer)]
        id: i32,
        #[diesel(sql_type=BigInt)]
        revision: i64,
    }
    let draft = diesel::sql_query("SELECT id,revision FROM workspace_drafts WHERE workspace_id=?1")
        .bind::<Integer, _>(workspace.id)
        .get_result::<Draft>(conn)
        .optional()?;
    if mode == super::DraftApplicationMode::ReplaceAll
        && draft.is_some()
        && expected_draft.is_none()
    {
        return Err(RepoError::InvalidState("draft_revision_conflict".into()));
    }
    if let Some(expected) = expected_draft {
        let actual = draft.as_ref().map(|draft| DraftRef {
            id: draft.id,
            revision: draft.revision,
        });
        if actual != Some(expected) {
            return Err(RepoError::InvalidState("draft_revision_conflict".into()));
        }
    }
    let (draft_id, revision) = if let Some(draft) = draft {
        (draft.id, draft.revision + 1)
    } else {
        diesel::sql_query("INSERT INTO workspace_drafts(workspace_id,base_version_id,revision,diagnostics_json,created_at_ms,updated_at_ms) VALUES(?1,?2,1,'{}',?3,?3)")
                        .bind::<Integer,_>(workspace.id).bind::<diesel::sql_types::Nullable<Integer>,_>(workspace.active_version_id).bind::<BigInt,_>(now_ms).execute(conn)?;
        let id =
            diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()")).get_result(conn)?;
        if let Some(version_id) = workspace
            .active_version_id
            .filter(|_| mode == super::DraftApplicationMode::MergeWithActiveBase)
        {
            diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) SELECT ?1,table_name,artifact_id,schema_json,row_count,content_hash,?2 FROM model_dataframes WHERE version_id=?3")
                            .bind::<Integer,_>(id).bind::<BigInt,_>(now_ms).bind::<Integer,_>(version_id).execute(conn)?;
            let artifact_ids = schema::model_dataframes::table
                .filter(schema::model_dataframes::version_id.eq(version_id))
                .select(schema::model_dataframes::artifact_id)
                .load::<i32>(conn)?;
            super::super::artifact_refs::retain_refs(conn, scope_id, artifact_ids)?;
        }
        (id, 1)
    };
    let mut release = Vec::new();
    if mode == super::DraftApplicationMode::ReplaceAll {
        release = schema::draft_dataframes::table
            .filter(schema::draft_dataframes::draft_id.eq(draft_id))
            .select(schema::draft_dataframes::artifact_id)
            .load(conn)?;
        diesel::delete(
            schema::draft_dataframes::table.filter(schema::draft_dataframes::draft_id.eq(draft_id)),
        )
        .execute(conn)?;
    }
    for candidate in &candidates {
        let table_name = candidate
            .normalized_alias
            .clone()
            .or_else(|| candidate.proposed_alias.clone())
            .ok_or_else(|| RepoError::InvalidState("模型候选缺少表名".into()))?;
        if table_name.eq_ignore_ascii_case("para") {
            return Err(RepoError::InvalidState("para 不能进入模型草稿".into()));
        }
        let artifact_id = candidate
            .normalized_artifact_id
            .ok_or_else(|| RepoError::InvalidState("模型候选缺少 artifact".into()))?;
        let content_hash = schema::artifacts::table
            .find(artifact_id)
            .select(schema::artifacts::sha256)
            .first::<String>(conn)?;
        let old = schema::draft_dataframes::table
            .filter(schema::draft_dataframes::draft_id.eq(draft_id))
            .filter(schema::draft_dataframes::table_name.eq(&table_name))
            .select(schema::draft_dataframes::artifact_id)
            .first::<i32>(conn)
            .optional()?;
        diesel::delete(
            schema::draft_dataframes::table
                .filter(schema::draft_dataframes::draft_id.eq(draft_id))
                .filter(schema::draft_dataframes::table_name.eq(&table_name)),
        )
        .execute(conn)?;
        diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                        .bind::<Integer,_>(draft_id).bind::<Text,_>(&table_name).bind::<Integer,_>(artifact_id)
                        .bind::<Text,_>(candidate.schema_json.clone().ok_or_else(|| RepoError::InvalidState("模型候选缺少 schema".into()))?)
                        .bind::<BigInt,_>(candidate.row_count.ok_or_else(|| RepoError::InvalidState("模型候选缺少行数".into()))?)
                        .bind::<Text,_>(content_hash)
                        .bind::<BigInt,_>(now_ms).execute(conn)?;
        if let Some(old) = old {
            release.push(old);
        }
        diesel::update(schema::upload_candidates::table.find(&candidate.id))
            .set(schema::upload_candidates::normalized_artifact_id.eq(Option::<i32>::None))
            .execute(conn)?;
    }
    diesel::update(schema::workspace_drafts::table.find(draft_id))
        .set((
            schema::workspace_drafts::revision.eq(revision),
            schema::workspace_drafts::updated_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    super::super::artifact_refs::release_refs(conn, scope_id, release, now_ms)?;
    let receipt = serde_json::json!({"draftId":draft_id,"draftRevision":revision,"tableCount":candidates.len()}).to_string();
    diesel::update(schema::upload_batches::table.find(batch_id))
        .set((
            schema::upload_batches::model_publish_status.eq("published"),
            schema::upload_batches::model_receipt_json.eq(Some(receipt)),
            schema::upload_batches::model_diagnostics_json.eq("[]"),
            schema::upload_batches::updated_at_ms.eq(now_ms),
        ))
        .execute(conn)?;
    Ok(())
}
