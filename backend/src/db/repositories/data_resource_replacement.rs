use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use crate::auth::UserId;
use crate::db::models::{DataResourceRecord, DataResourceReplacementPreviewRecord};
use crate::db::schema;

use super::{DataResourceRepo, RepoError, RepoResult, owned_resource};

#[derive(Debug, Clone)]
pub enum StructuredPreviewConfirmation {
    Published(Box<DataResourceRecord>),
    Cancelled,
    Rejected { code: &'static str },
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

impl DataResourceRepo {
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
                super::super::artifact_refs::release_refs(
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
                    super::super::artifact_refs::release_refs(
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
                    super::super::artifact_refs::release_refs(
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
                    super::super::artifact_refs::release_refs(
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
                    super::super::artifact_refs::release_refs(
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
                    super::super::artifact_refs::release_refs(
                        conn,
                        scope_id,
                        artifact_ids,
                        now_ms,
                    )?;
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
}
