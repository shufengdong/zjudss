use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::auth::UserId;
use crate::db::models::DataResourceRecord;
use crate::db::schema;

use super::{DataResourceRepo, RepoError, RepoResult, owned_resource};

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

pub struct DeletionPreviewInput {
    pub resource_id: String,
    pub token_hash: String,
    pub impact_hash: String,
    pub impact_json: String,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

impl DataResourceRepo {
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
                super::super::artifact_refs::release_refs(
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
    #[diesel(sql_type = BigInt)]
    value: i64,
}
